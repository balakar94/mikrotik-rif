//! The capture index: every part recorded in one support file.
//!
//! Indexing is deliberately split from expansion. Reading a capture transcodes
//! just enough of each part to learn its label and keeps the payload
//! compressed; only [`Capture::read`] inflates a payload, and only for the part
//! the caller asked for. That keeps a multi-hundred-megabyte capture cheap to
//! open and keeps peak memory proportional to a single part, not the file.

use std::collections::HashMap;

use crate::parser::codec;
use crate::parser::deflate;
use crate::parser::error::RifError;
use crate::parser::limits::CaptureLimits;
use crate::parser::scanner::{self, ContainerNote, PartSpan};

/// One named part of a capture.
#[derive(Debug)]
pub struct Part {
    label: String,
    ordinal: usize,
    payload: Vec<u8>,
    lossy_label: bool,
    fault: Option<String>,
}

impl Part {
    /// Build a placeholder for a part that could not be indexed.
    fn unreadable(position: usize, reason: String) -> Self {
        Self {
            label: format!("<unreadable part {position}>"),
            ordinal: 0,
            payload: Vec::new(),
            lossy_label: true,
            fault: Some(reason),
        }
    }

    /// Human-readable name declared by the router.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Zero-based disambiguator when several parts share the same label.
    #[must_use]
    pub const fn ordinal(&self) -> usize {
        self.ordinal
    }

    /// Whether the label required lossy UTF-8 conversion.
    #[must_use]
    pub const fn is_lossy_label(&self) -> bool {
        self.lossy_label
    }

    /// Compressed payload size, in bytes.
    #[must_use]
    pub fn compressed_len(&self) -> usize {
        self.payload.len()
    }

    /// Whether the part can be expanded.
    #[must_use]
    pub const fn is_readable(&self) -> bool {
        self.fault.is_none()
    }

    /// Why the part could not be indexed, when it could not be.
    #[must_use]
    pub fn fault(&self) -> Option<&str> {
        self.fault.as_deref()
    }

    /// Raw compressed payload, kept for diagnostics and raw export.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Expanded text of one part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartText {
    /// Decoded text, with RouterOS carriage returns left intact.
    pub text: String,

    /// Whether decoding had to replace invalid UTF-8 sequences.
    pub lossy: bool,
}

/// An indexed support capture.
#[derive(Debug)]
pub struct Capture {
    parts: Vec<Part>,
    notes: Vec<ContainerNote>,
}

impl Capture {
    /// Index every part in a capture buffer without expanding any payload.
    ///
    /// # Errors
    ///
    /// Container-level failures abort: an unterminated part, an exceeded budget,
    /// or a marker problem when strict mode is enabled. Per-part problems are
    /// recorded as [`Part::fault`] so the rest of the capture stays usable.
    pub fn from_bytes(source: &[u8], limits: &CaptureLimits) -> Result<Self, RifError> {
        let mut notes = Vec::new();
        let spans = scanner::locate_parts(source, limits, &mut notes)?;

        if spans.len() > limits.max_parts {
            return Err(RifError::TooManyParts {
                found: spans.len(),
                limit: limits.max_parts,
            });
        }

        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut parts = Vec::with_capacity(spans.len());

        for (position, &PartSpan { start, end }) in spans.iter().enumerate() {
            let span_len = end.saturating_sub(start);
            if span_len > limits.max_span_bytes {
                return Err(RifError::PartSpanTooLarge {
                    offset: start,
                    limit: limits.max_span_bytes,
                });
            }

            match index_part(&source[start..end], position) {
                Ok((label, payload, lossy_label)) => {
                    if limits.strict_labels && lossy_label {
                        parts.push(Part::unreadable(
                            position,
                            "label is not valid UTF-8 and strict labels are enabled".to_owned(),
                        ));
                        continue;
                    }
                    let counter = seen.entry(label.clone()).or_insert(0);
                    let ordinal = *counter;
                    *counter += 1;
                    parts.push(Part {
                        label,
                        ordinal,
                        payload,
                        lossy_label,
                        fault: None,
                    });
                }
                Err(error) => parts.push(Part::unreadable(position, error.to_string())),
            }
        }

        Ok(Self { parts, notes })
    }

    /// Every part, in the order the router wrote them.
    #[must_use]
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    /// Structural oddities noticed while scanning, if any.
    #[must_use]
    pub fn notes(&self) -> &[ContainerNote] {
        &self.notes
    }

    /// Number of parts, readable or not.
    #[must_use]
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Whether the capture holds no parts at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Indices of every part carrying `label`, in file order.
    #[must_use]
    pub fn indices_named(&self, label: &str) -> Vec<usize> {
        self.parts
            .iter()
            .enumerate()
            .filter(|(_, part)| part.label == label)
            .map(|(index, _)| index)
            .collect()
    }

    /// Expand one part into text.
    ///
    /// # Errors
    ///
    /// Fails when the index is out of range, when the part was flagged during
    /// indexing, when decompression exceeds the budget, or when the payload is
    /// not a valid zlib stream.
    pub fn read(&self, index: usize, limits: &CaptureLimits) -> Result<PartText, RifError> {
        let part = self
            .parts
            .get(index)
            .ok_or(RifError::NoSuchPart { index })?;
        if part.fault.is_some() {
            return Err(RifError::PartFlagged {
                label: part.label.clone(),
            });
        }

        let bytes = deflate::expand(&part.payload, limits.max_part_bytes)?;
        let (text, lossy) = match String::from_utf8(bytes) {
            Ok(text) => (text, false),
            Err(error) => (String::from_utf8_lossy(error.as_bytes()).into_owned(), true),
        };
        Ok(PartText { text, lossy })
    }
}

/// Transcode one part body and split it into label and compressed payload.
fn index_part(body: &[u8], position: usize) -> Result<(String, Vec<u8>, bool), RifError> {
    let raw = codec::unpack(body)?;

    let separator = raw
        .iter()
        .position(|&byte| byte == 0)
        .ok_or(RifError::MissingLabelSeparator)?;
    if separator == 0 {
        return Err(RifError::MissingLabelSeparator);
    }

    let (label_bytes, rest) = raw.split_at(separator);
    let payload = &rest[1..];
    if payload.is_empty() {
        return Err(RifError::EmptyPayload { position });
    }

    let (label, lossy) = match std::str::from_utf8(label_bytes) {
        Ok(label) => (label.to_owned(), false),
        Err(_) => (String::from_utf8_lossy(label_bytes).into_owned(), true),
    };

    Ok((label, payload.to_vec(), lossy))
}
