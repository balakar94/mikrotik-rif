//! The capture index: every part recorded in one support file.
//!
//! Indexing is deliberately split from expansion. Reading a capture transcodes
//! just enough of each part to learn its label and keeps the payload
//! compressed; only [`Capture::read`] inflates a payload, and only for the part
//! the caller asked for. That keeps a multi-hundred-megabyte capture cheap to
//! open and keeps peak memory proportional to a single part, not the file.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;

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
        let mut total_payload_bytes: u64 = 0;

        for (position, &PartSpan { start, end }) in spans.iter().enumerate() {
            let span_len = end.saturating_sub(start);
            if span_len > limits.max_span_bytes {
                return Err(RifError::PartSpanTooLarge {
                    offset: start,
                    limit: limits.max_span_bytes,
                });
            }

            let ceiling = u64::try_from(limits.max_total_payload_bytes).unwrap_or(u64::MAX);
            let remaining = ceiling.saturating_sub(total_payload_bytes);
            let budget = usize::try_from(remaining).unwrap_or(usize::MAX);
            match index_part(&source[start..end], position, budget) {
                Ok((label, payload, lossy_label, decoded_len)) => {
                    total_payload_bytes = total_payload_bytes.saturating_add(decoded_len);
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
                Err(RifError::BudgetExceeded { .. }) => {
                    // A global budget, not a damaged part: abort instead of
                    // downgrading to an unreadable placeholder. Report the
                    // configured ceiling rather than the remaining slice.
                    return Err(RifError::BudgetExceeded {
                        limit: limits.max_total_payload_bytes,
                    });
                }
                Err(error) => parts.push(Part::unreadable(position, error.to_string())),
            }
        }

        Ok(Self { parts, notes })
    }

    /// Index a capture arriving as a byte stream.
    ///
    /// The stream is buffered in memory, capped at
    /// [`CaptureLimits::max_total_payload_bytes`], and then indexed exactly as
    /// if the caller had collected it and passed it to
    /// [`Capture::from_bytes`].
    ///
    /// # Errors
    ///
    /// Fails with [`RifError::StreamRead`] when the stream cannot be buffered,
    /// with [`RifError::BudgetExceeded`] when the stream holds more than
    /// `max_total_payload_bytes` bytes, plus every [`Capture::from_bytes`]
    /// failure mode.
    pub fn from_reader<R: Read>(source: R, limits: &CaptureLimits) -> Result<Self, RifError> {
        let cap = u64::try_from(limits.max_total_payload_bytes).unwrap_or(u64::MAX);
        let mut bytes = Vec::new();
        source
            .take(cap.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| RifError::StreamRead { source })?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > cap {
            return Err(RifError::BudgetExceeded {
                limit: limits.max_total_payload_bytes,
            });
        }
        Self::from_bytes(&bytes, limits)
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
///
/// `budget` caps the transcoded byte count for this part; the caller passes
/// whatever is left of the capture-wide total budget. Also returns the
/// transcoded length so the caller can accumulate the running total.
fn index_part(
    body: &[u8],
    position: usize,
    budget: usize,
) -> Result<(String, Vec<u8>, bool, u64), RifError> {
    let raw = codec::unpack_capped(body, budget)?;
    let decoded_len = u64::try_from(raw.len()).unwrap_or(u64::MAX);

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

    Ok((label, payload.to_vec(), lossy, decoded_len))
}

// ── Pure view kernels ────────────────────────────────────────────────────────
// UI-agnostic selection helpers mirroring the workspace behaviour. They live
// here — next to the labels and part text they operate on — so they stay
// unit-tested without pulling in any UI toolkit. Wave2 wires `app.rs` and
// `workspace.rs` to delegate to them; until then they are intentionally
// unused outside tests.

/// Indices of `labels` matching `needle`.
///
/// The needle is trimmed and matched as a case-insensitive substring, exactly
/// like the module-rail filter; an empty needle selects every part.
#[allow(dead_code)]
#[must_use]
pub fn filter_parts(labels: &[String], needle: &str) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    labels
        .iter()
        .enumerate()
        .filter(|(_, label)| needle.is_empty() || label.to_lowercase().contains(&needle))
        .map(|(index, _)| index)
        .collect()
}

/// Line starts of `text` plus the longest line length in characters.
///
/// Line endings are excluded from the length, matching the width the reading
/// surface reserves for its horizontal scroll area.
#[allow(dead_code)]
#[must_use]
pub fn compute_view(text: &str) -> (Vec<usize>, usize) {
    let mut starts = vec![0];
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(offset + 1);
        }
    }
    let mut longest = 0;
    for (row, &start) in starts.iter().enumerate() {
        let end = starts.get(row + 1).copied().unwrap_or(text.len());
        let raw = &text[start..end];
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        longest = longest.max(line.chars().count());
    }
    (starts, longest)
}

/// Next match row after `cursor`, stepping `step` (`+1` forward, `-1` back)
/// with wraparound, mirroring the find-bar navigation.
///
/// Returns `None` when there are no matches; a zero step keeps the cursor
/// clamped into range.
#[allow(dead_code)]
#[must_use]
pub fn next_match(total: usize, cursor: usize, step: i32) -> Option<usize> {
    if total == 0 {
        return None;
    }
    match step.cmp(&0) {
        Ordering::Greater => Some((cursor + 1) % total),
        Ordering::Less => Some(cursor.checked_sub(1).unwrap_or(total - 1)),
        Ordering::Equal => Some(cursor.min(total - 1)),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;
    use crate::parser::scanner::{CLOSE_MARKER, OPEN_MARKER};

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Deterministic incompressible bytes: a 64-bit LCG spilling little-endian
    /// words, so zlib cannot find any repetition to exploit.
    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            out.extend_from_slice(&state.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    /// Encode one part the way the router would: label, NUL, zlib payload.
    /// The label is raw bytes so tests can feed non-UTF-8 sequences.
    fn encode_part(label: &[u8], body: &[u8]) -> Vec<u8> {
        let mut plain = label.to_vec();
        plain.push(0);
        plain.extend_from_slice(&deflate(body));
        codec::pack(&plain)
    }

    /// Wrap encoded parts in the container markers.
    fn wrap(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut source = Vec::new();
        for part in parts {
            source.extend_from_slice(OPEN_MARKER);
            source.push(b'\n');
            source.extend_from_slice(part);
            source.push(b'\n');
            source.extend_from_slice(CLOSE_MARKER);
            source.push(b'\n');
        }
        source
    }

    #[test]
    fn reader_facade_indexes_exactly_like_bytes() {
        let source = wrap(&[encode_part(b"export", b"hello\n")]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_reader(source.as_slice(), &limits).expect("stream must index");
        assert_eq!(capture.len(), 1);
        assert_eq!(capture.read(0, &limits).unwrap().text, "hello\n");
    }

    #[test]
    fn zip_bomb_is_contained_by_the_expansion_budget() {
        let bomb = vec![0u8; 1024 * 1024];
        let source = wrap(&[encode_part(b"big", &bomb)]);
        let capture =
            Capture::from_bytes(&source, &CaptureLimits::default()).expect("index stays cheap");
        assert_eq!(capture.len(), 1);

        let tight = CaptureLimits {
            max_part_bytes: 1024,
            ..CaptureLimits::default()
        };
        let error = capture.read(0, &tight).expect_err("limit must trip");
        assert!(matches!(error, RifError::PayloadAboveLimit { limit: 1024 }));
    }

    #[test]
    fn total_payload_budget_accumulates_across_parts() {
        let first = pseudo_random(600 * 1024, 0x1234_5678);
        let second = pseudo_random(600 * 1024, 0x9ABC_DEF0);
        let source = wrap(&[encode_part(b"one", &first), encode_part(b"two", &second)]);
        let limits = CaptureLimits {
            max_total_payload_bytes: 1024 * 1024,
            ..CaptureLimits::default()
        };
        let error = Capture::from_bytes(&source, &limits).expect_err("budget must trip");
        assert!(matches!(
            error,
            RifError::BudgetExceeded { limit } if limit == 1024 * 1024
        ));
    }

    #[test]
    fn single_line_past_the_line_budget_aborts() {
        let source = vec![b'A'; 64];
        let limits = CaptureLimits {
            max_line_bytes: 16,
            ..CaptureLimits::default()
        };
        let error = Capture::from_bytes(&source, &limits).expect_err("line must trip");
        assert!(matches!(error, RifError::LineTooLong { limit: 16, .. }));
    }

    #[test]
    fn non_utf8_label_is_lossy_unless_strict() {
        let source = wrap(&[encode_part(b"fo\xff\xfe", b"hello\n")]);

        let capture =
            Capture::from_bytes(&source, &CaptureLimits::default()).expect("capture must index");
        assert!(capture.parts()[0].is_readable());
        assert!(capture.parts()[0].is_lossy_label());
        assert_eq!(
            capture.read(0, &CaptureLimits::default()).unwrap().text,
            "hello\n"
        );

        let strict = CaptureLimits {
            strict_labels: true,
            ..CaptureLimits::default()
        };
        let flagged = Capture::from_bytes(&source, &strict).expect("container stays readable");
        assert!(!flagged.parts()[0].is_readable());
        assert!(flagged.parts()[0].fault().is_some());
        let error = flagged
            .read(0, &strict)
            .expect_err("flagged part must not expand");
        assert!(matches!(error, RifError::PartFlagged { .. }));
    }

    #[test]
    fn filter_parts_mirrors_the_module_rail() {
        let labels = [
            "Export".to_owned(),
            "other".to_owned(),
            "my export".to_owned(),
        ];
        assert_eq!(filter_parts(&labels, ""), [0, 1, 2]);
        assert_eq!(filter_parts(&labels, "  EXPORT "), [0, 2]);
        assert_eq!(filter_parts(&labels, "other"), [1]);
        assert!(filter_parts(&labels, "missing").is_empty());
        assert!(filter_parts(&[], "x").is_empty());
    }

    #[test]
    fn compute_view_indexes_lines_and_measures_width() {
        let (starts, longest) = compute_view("a\r\nlonger line\nx");
        assert_eq!(starts, [0, 3, 15]);
        assert_eq!(longest, 11);
        assert_eq!(compute_view(""), (vec![0], 0));
    }

    #[test]
    fn next_match_steps_with_wraparound() {
        assert_eq!(next_match(0, 0, 1), None);
        assert_eq!(next_match(3, 0, 1), Some(1));
        assert_eq!(next_match(3, 2, 1), Some(0));
        assert_eq!(next_match(3, 0, -1), Some(2));
        assert_eq!(next_match(3, 1, -1), Some(0));
        assert_eq!(next_match(3, 9, 0), Some(2));
    }
}
