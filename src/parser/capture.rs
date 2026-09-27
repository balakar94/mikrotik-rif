//! The capture index: every part recorded in one support file.
//!
//! Indexing is deliberately split from expansion. Reading a capture transcodes
//! each part body once into a single buffer, records the offset at which the
//! compressed payload begins (the label and its NUL separator stay in the same
//! buffer), and leaves the payload compressed. Only [`Capture::read`] and its
//! siblings inflate a payload, and only for the part the caller asked for. That
//! keeps a multi-hundred-megabyte capture cheap to open and keeps peak memory
//! proportional to the parts, not to a second copy of each payload.
//!
//! The label must be known to identify a part, and it is separated from the
//! payload only by a `NUL` byte inside the transcoded stream, so the label
//! cannot be recovered without transcoding the whole body. Recording the
//! payload offset keeps that payload as a borrow into the single transcoded
//! buffer instead of copying it into a second allocation.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::sync::Arc;

use crate::parser::codec;
use crate::parser::deflate;
use crate::parser::error::RifError;
use crate::parser::limits::{Cancel, CaptureLimits};
use crate::parser::scanner::{self, ContainerNote, PartSpan};

/// One named part of a capture.
#[derive(Debug)]
pub struct Part {
    label: String,
    ordinal: usize,
    /// Transcoded body: label bytes, the `NUL` separator, then the compressed
    /// payload. Kept whole so the payload is a borrow rather than a copy.
    decoded: Vec<u8>,
    /// Offset of the first payload byte within `decoded`.
    payload_start: usize,
    lossy_label: bool,
    fault: Option<String>,
}

impl Part {
    /// Build a placeholder for a part that could not be indexed.
    fn unreadable(position: usize, reason: String) -> Self {
        Self {
            label: format!("<unreadable part {position}>"),
            ordinal: 0,
            decoded: Vec::new(),
            payload_start: 0,
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
    ///
    /// The payload is the transcoded zlib stream after the label separator, so
    /// this is the size handed to the expander, not the raw span length.
    #[must_use]
    pub fn compressed_len(&self) -> usize {
        self.decoded.len().saturating_sub(self.payload_start)
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
    ///
    /// Returns the bytes after the label's `NUL` separator, or an empty slice
    /// for a part that could not be indexed.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        self.decoded.get(self.payload_start..).unwrap_or_default()
    }

    /// Decoded buffer length and payload offset, so tests can assert that the
    /// payload is a suffix of the one indexing allocation.
    #[cfg(test)]
    fn storage(&self) -> (usize, usize) {
        (self.decoded.len(), self.payload_start)
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

impl PartText {
    /// Heap bytes retained by the text, used for the cache byte budget.
    fn retained_bytes(&self) -> usize {
        self.text.capacity()
    }
}

/// Number of parts a [`PartCache`] holds by default.
pub const DEFAULT_PART_CACHE_ENTRIES: usize = 8;

/// Byte budget a [`PartCache`] holds by default.
pub const DEFAULT_PART_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// One cached part text plus the key and budgets it was stored under.
#[derive(Debug)]
struct CacheEntry {
    index: usize,
    text: Arc<PartText>,
    /// Decompressed byte count before UTF-8 decoding, so a later hit can be
    /// checked against a tighter [`CaptureLimits::max_part_bytes`] exactly even
    /// when the text was decoded lossily.
    plain_len: usize,
}

/// Bounded least-recently-used cache of expanded parts.
///
/// Re-expanding a part is the most expensive step a caller can repeat, and a
/// viewer typically re-reads the same part on every redraw. The cache stores at
/// most [`Self::max_entries`] entries and at most [`Self::max_bytes`] of
/// retained text; beyond either bound the least recently used entry is evicted.
///
/// A text larger than the byte budget is never cached, so a single huge part
/// cannot flush the cache into allocating on every use. Entries are shared as
/// [`Arc`], so a cache hit costs no copy and the same text can be handed to
/// several callers.
///
/// Entries are keyed by part index alone. A cache must therefore be used with
/// the single [`Capture`] it was filled from; sharing one across different
/// captures would return text belonging to the wrong part.
#[derive(Debug)]
pub struct PartCache {
    entries: VecDeque<CacheEntry>,
    max_entries: usize,
    max_bytes: usize,
    bytes: usize,
}

impl PartCache {
    /// Create a cache holding at most `max_entries` entries and `max_bytes` of
    /// retained text.
    #[must_use]
    pub const fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries,
            max_bytes,
            bytes: 0,
        }
    }

    /// Look up a part, marking it as most recently used on a hit.
    #[must_use]
    pub fn get(&mut self, index: usize) -> Option<Arc<PartText>> {
        self.get_entry(index).map(|(text, _)| text)
    }

    /// Look up a part together with its decompressed byte count, marking it as
    /// most recently used on a hit.
    fn get_entry(&mut self, index: usize) -> Option<(Arc<PartText>, usize)> {
        let position = self.entries.iter().position(|entry| entry.index == index)?;
        let entry = self.entries.remove(position)?;
        let text = Arc::clone(&entry.text);
        let plain_len = entry.plain_len;
        self.entries.push_back(entry);
        Some((text, plain_len))
    }

    /// Insert or replace the text for `index`, evicting least-recently-used
    /// entries until both bounds hold again.
    ///
    /// `plain_len` is the decompressed byte count the text was decoded from; it
    /// is stored so a later hit can be validated against a tightened
    /// [`CaptureLimits::max_part_bytes`] even when decoding was lossy. A text
    /// whose retained size exceeds the byte budget is skipped: caching it would
    /// immediately evict everything else and still not fit.
    pub fn insert(&mut self, index: usize, text: Arc<PartText>, plain_len: usize) {
        if self.max_entries == 0 || self.max_bytes == 0 {
            return;
        }
        let size = text.retained_bytes();
        if size > self.max_bytes {
            return;
        }
        if let Some(position) = self.entries.iter().position(|entry| entry.index == index)
            && let Some(old) = self.entries.remove(position)
        {
            self.bytes = self.bytes.saturating_sub(old.text.retained_bytes());
        }
        self.bytes = self.bytes.saturating_add(size);
        self.entries.push_back(CacheEntry {
            index,
            text,
            plain_len,
        });
        while self.entries.len() > self.max_entries || self.bytes > self.max_bytes {
            let Some(evicted) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(evicted.text.retained_bytes());
        }
    }

    /// Number of cached parts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no parts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Retained text bytes currently accounted for.
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Configured entry bound.
    #[must_use]
    pub const fn max_entries(&self) -> usize {
        self.max_entries
    }

    /// Configured byte bound.
    #[must_use]
    pub const fn max_bytes(&self) -> usize {
        self.max_bytes
    }

    /// Drop every cached part.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

impl Default for PartCache {
    fn default() -> Self {
        Self::new(DEFAULT_PART_CACHE_ENTRIES, DEFAULT_PART_CACHE_BYTES)
    }
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
    /// This is the non-cancellable convenience wrapper around
    /// [`Capture::from_bytes_cancellable`]; it uses a never-cancelled token.
    ///
    /// # Errors
    ///
    /// Container-level failures abort: an unterminated part, an exceeded budget,
    /// or a marker problem when strict mode is enabled. Per-part problems are
    /// recorded as [`Part::fault`] so the rest of the capture stays usable.
    pub fn from_bytes(source: &[u8], limits: &CaptureLimits) -> Result<Self, RifError> {
        Self::from_bytes_cancellable(source, limits, &Cancel::default())
    }

    /// Index a capture buffer, honouring a cancellation token.
    ///
    /// The token is checked per scan line while the container is walked and
    /// once per part while bodies are transcoded. A raised token aborts with
    /// [`RifError::Cancelled`] and is never downgraded to a per-part fault, so
    /// the caller cannot mistake an aborted index for a complete one.
    ///
    /// # Errors
    ///
    /// Everything [`Capture::from_bytes`] reports, plus [`RifError::Cancelled`]
    /// when the token is raised.
    pub fn from_bytes_cancellable(
        source: &[u8],
        limits: &CaptureLimits,
        cancel: &Cancel,
    ) -> Result<Self, RifError> {
        let mut notes = Vec::new();
        let spans = scanner::locate_parts_cancellable(source, limits, &mut notes, cancel)?;

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
            if cancel.is_cancelled() {
                return Err(RifError::Cancelled);
            }
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
                Ok(indexed) => {
                    total_payload_bytes = total_payload_bytes.saturating_add(indexed.decoded_len);
                    if limits.strict_labels && indexed.lossy_label {
                        parts.push(Part::unreadable(
                            position,
                            "label is not valid UTF-8 and strict labels are enabled".to_owned(),
                        ));
                        continue;
                    }
                    let counter = seen.entry(indexed.label.clone()).or_insert(0);
                    let ordinal = *counter;
                    *counter += 1;
                    parts.push(Part {
                        label: indexed.label,
                        ordinal,
                        decoded: indexed.decoded,
                        payload_start: indexed.payload_start,
                        lossy_label: indexed.lossy_label,
                        fault: None,
                    });
                }
                Err(RifError::Cancelled) => return Err(RifError::Cancelled),
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

    /// Index of the first part carrying `label`, in file order.
    ///
    /// The match is exact (`==` on the stored label); use
    /// [`filter_parts`](crate::parser::capture::filter_parts) for the
    /// case-insensitive substring search of the module rail. Returns `None`
    /// when no part carries `label`. Equivalent to
    /// `self.indices_named(label).first().copied()`, but stops at the first
    /// hit instead of scanning the whole capture.
    #[must_use]
    pub fn find_first_named(&self, label: &str) -> Option<usize> {
        self.parts.iter().position(|part| part.label == label)
    }

    /// Expand one part into text.
    ///
    /// This expands directly through [`Capture::read_bytes`] with a
    /// never-cancelled token; the result is always freshly expanded and never
    /// cached. Use [`Capture::read_cached`] when a part is likely to be read
    /// repeatedly.
    ///
    /// # Errors
    ///
    /// Fails when the index is out of range, when the part was flagged during
    /// indexing, when decompression exceeds [`CaptureLimits::max_part_bytes`],
    /// or when the payload is not a valid zlib stream.
    pub fn read(&self, index: usize, limits: &CaptureLimits) -> Result<PartText, RifError> {
        let bytes = self.read_bytes(index, limits, &Cancel::default())?;
        Ok(decode_text(bytes))
    }

    /// Expand one part into its raw decompressed bytes.
    ///
    /// This is the primitive the text paths build on: UTF-8 decoding is a
    /// separate step, so callers that want the bytes (for export or a hex view)
    /// pay nothing for it. The output is budgeted by
    /// [`CaptureLimits::max_part_bytes`].
    ///
    /// # Errors
    ///
    /// Fails when the index is out of range, when the part was flagged during
    /// indexing, when decompression exceeds the budget, when the token is
    /// raised, or when the payload is not a valid zlib stream.
    pub fn read_bytes(
        &self,
        index: usize,
        limits: &CaptureLimits,
        cancel: &Cancel,
    ) -> Result<Vec<u8>, RifError> {
        let part = self
            .parts
            .get(index)
            .ok_or(RifError::NoSuchPart { index })?;
        if part.fault.is_some() {
            return Err(RifError::PartFlagged {
                label: part.label.clone(),
            });
        }
        deflate::expand_cancellable(part.payload(), limits.max_part_bytes, cancel)
    }

    /// Expand one part into shared text, consulting and filling `cache`.
    ///
    /// A hit returns the cached [`Arc`] with no work. A miss expands the part,
    /// decodes it to UTF-8, stores the result, and returns it. The token is
    /// checked while expanding on a miss; a cached hit never touches it. A hit
    /// whose decompressed size exceeds the caller's current
    /// [`CaptureLimits::max_part_bytes`] is rejected instead of returned, so a
    /// cache carried across different limits cannot bypass the budget.
    ///
    /// `cache` is keyed by part index, so it must be the cache for `self` (see
    /// [`PartCache`]).
    ///
    /// # Errors
    ///
    /// Everything [`Capture::read_bytes`] reports. A cancellation during a miss
    /// leaves the cache unchanged.
    pub fn read_cached(
        &self,
        index: usize,
        limits: &CaptureLimits,
        cache: &mut PartCache,
        cancel: &Cancel,
    ) -> Result<Arc<PartText>, RifError> {
        if let Some((cached, plain_len)) = cache.get_entry(index) {
            if plain_len > limits.max_part_bytes {
                return Err(RifError::PayloadAboveLimit {
                    limit: limits.max_part_bytes,
                });
            }
            return Ok(cached);
        }
        let bytes = self.read_bytes(index, limits, cancel)?;
        let plain_len = bytes.len();
        let text = Arc::new(decode_text(bytes));
        cache.insert(index, Arc::clone(&text), plain_len);
        Ok(text)
    }

    /// Expand one part into shared text, honouring cancellation and using
    /// `cache`.
    ///
    /// This is an alias of [`Capture::read_cached`], named for call sites that
    /// think in terms of cancellation.
    ///
    /// # Errors
    ///
    /// Same as [`Capture::read_cached`].
    pub fn read_cancellable(
        &self,
        index: usize,
        limits: &CaptureLimits,
        cache: &mut PartCache,
        cancel: &Cancel,
    ) -> Result<Arc<PartText>, RifError> {
        self.read_cached(index, limits, cache, cancel)
    }
}

/// Split decompressed bytes into text, replacing invalid UTF-8 lossily.
fn decode_text(bytes: Vec<u8>) -> PartText {
    match String::from_utf8(bytes) {
        Ok(text) => PartText { text, lossy: false },
        Err(error) => PartText {
            text: String::from_utf8_lossy(error.as_bytes()).into_owned(),
            lossy: true,
        },
    }
}

/// One indexed body: the transcoded buffer plus where its payload starts.
struct IndexedPart {
    label: String,
    decoded: Vec<u8>,
    payload_start: usize,
    lossy_label: bool,
    decoded_len: u64,
}

/// Transcode one part body and split it into label and compressed payload.
///
/// `budget` caps the transcoded byte count for this part; the caller passes
/// whatever is left of the capture-wide total budget. The transcoded buffer is
/// returned whole with the payload offset recorded, so the payload is a borrow
/// into the buffer instead of a second allocation. The transcoded length is
/// also returned so the caller can accumulate the running total.
///
/// The label and separator bytes stay in the returned buffer; they are a small
/// fixed prefix compared with the payload and dropping them would force a
/// shifting copy that this design exists to avoid.
fn index_part(body: &[u8], position: usize, budget: usize) -> Result<IndexedPart, RifError> {
    let decoded = codec::unpack_capped(body, budget)?;
    let decoded_len = u64::try_from(decoded.len()).unwrap_or(u64::MAX);

    let separator = decoded
        .iter()
        .position(|&byte| byte == 0)
        .ok_or(RifError::MissingLabelSeparator)?;
    if separator == 0 {
        return Err(RifError::MissingLabelSeparator);
    }

    let payload_start = separator + 1;
    if payload_start >= decoded.len() {
        return Err(RifError::EmptyPayload { position });
    }

    let label_bytes = &decoded[..separator];
    let (label, lossy_label) = match std::str::from_utf8(label_bytes) {
        Ok(label) => (label.to_owned(), false),
        Err(_) => (String::from_utf8_lossy(label_bytes).into_owned(), true),
    };

    Ok(IndexedPart {
        label,
        decoded,
        payload_start,
        lossy_label,
        decoded_len,
    })
}

// ── Pure view kernels ────────────────────────────────────────────────────────
// UI-agnostic selection helpers mirroring the workspace behaviour. They live
// here — next to the labels and part text they operate on — so they stay
// unit-tested without pulling in any UI toolkit. They are re-exported from
// [`crate::parser`], which is what the interface consumes.

/// Indices of `labels` matching `needle`.
///
/// The needle is trimmed and matched as a case-insensitive substring, exactly
/// like the module-rail filter; an empty needle selects every part.
#[must_use]
pub fn filter_parts(labels: &[String], needle: &str) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    if needle.is_ascii() {
        let needle_bytes = needle.as_bytes();
        labels
            .iter()
            .enumerate()
            .filter(|(_, label)| {
                if needle.is_empty() {
                    return true;
                }
                if label.is_ascii() {
                    ascii_contains_ci(label.as_bytes(), needle_bytes)
                } else {
                    label.to_lowercase().contains(&needle)
                }
            })
            .map(|(index, _)| index)
            .collect()
    } else {
        labels
            .iter()
            .enumerate()
            .filter(|(_, label)| needle.is_empty() || label.to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect()
    }
}

/// Case-insensitive ASCII substring search without allocating.
///
/// Both slices must already be ASCII (the caller checks `str::is_ascii`);
/// bytes are compared case-insensitively during the scan, so no lowered copy
/// of the haystack is ever built.
fn ascii_contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    let first = needle[0].to_ascii_lowercase();
    haystack
        .iter()
        .enumerate()
        .filter(|(_, byte)| byte.to_ascii_lowercase() == first)
        .any(|(start, _)| {
            haystack.len() - start >= needle.len()
                && haystack[start..start + needle.len()]
                    .iter()
                    .zip(needle.iter())
                    .all(|(&left, &right)| left.eq_ignore_ascii_case(&right))
        })
}

/// Line starts of `text` plus the longest line length in characters.
///
/// Line endings are excluded from the length, matching the width the reading
/// surface reserves for its horizontal scroll area.
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
    use std::sync::Arc;

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
    fn filter_parts_matches_unicode_case_insensitively() {
        // Non-ASCII needles (or labels) fall back to `to_lowercase`, so Ä/ä
        // must match exactly like the previous allocation-heavy path.
        let labels = ["Äpfel".to_owned(), "BANANE".to_owned(), "apfel".to_owned()];
        assert_eq!(filter_parts(&labels, "ä"), [0]);
        assert_eq!(filter_parts(&labels, "Ä"), [0]);
        assert_eq!(filter_parts(&labels, "banane"), [1]);
        assert_eq!(filter_parts(&labels, "apfel"), [2]);
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

    #[test]
    fn capture_types_are_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Capture>();
        assert_send_sync::<Part>();
        assert_send_sync::<PartText>();
        assert_send_sync::<PartCache>();
        assert_send_sync::<Cancel>();
    }

    #[test]
    fn indexed_payload_is_a_suffix_of_one_decoded_buffer() {
        let payload = b"payload bytes\n";
        let source = wrap(&[encode_part(b"label", payload)]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let part = &capture.parts()[0];

        let (decoded_len, payload_start) = part.storage();
        assert_eq!(
            payload_start,
            part.label().len() + 1,
            "payload starts right after the label and its NUL separator"
        );
        assert_eq!(
            payload_start + part.compressed_len(),
            decoded_len,
            "the payload must extend to the end of the one decoded buffer"
        );
        // The transcoder zero-pads the stream to a whole group; `payload` is the
        // zlib stream followed by that padding, all inside the same buffer.
        let expected = deflate(payload);
        assert!(part.payload().starts_with(&expected));
        assert!(
            part.payload()[expected.len()..]
                .iter()
                .all(|&byte| byte == 0),
            "only zero padding may follow the zlib stream"
        );
    }

    #[test]
    fn read_bytes_is_the_primitive_the_text_path_builds_on() {
        let body = b"ok\xff\xfe";
        let source = wrap(&[encode_part(b"label", body)]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();

        let raw = capture.read_bytes(0, &limits, &Cancel::new()).unwrap();
        assert_eq!(raw, body);

        let text = capture.read(0, &limits).unwrap();
        assert!(text.lossy);
        assert_eq!(text.text, "ok\u{fffd}\u{fffd}");
    }

    #[test]
    fn cancelled_indexing_aborts_and_is_not_downgraded() {
        let source = wrap(&[encode_part(b"label", b"payload\n")]);
        let cancel = Cancel::new();
        cancel.cancel();
        let error = Capture::from_bytes_cancellable(&source, &CaptureLimits::default(), &cancel)
            .expect_err("a cancelled index must abort");
        assert!(matches!(error, RifError::Cancelled));
    }

    #[test]
    fn cancelled_expansion_aborts_the_read_and_caches_nothing() {
        let source = wrap(&[encode_part(b"label", b"payload\n")]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let cancel = Cancel::new();
        cancel.cancel();

        let error = capture
            .read_bytes(0, &limits, &cancel)
            .expect_err("a cancelled read must abort");
        assert!(matches!(error, RifError::Cancelled));

        let mut cache = PartCache::default();
        let error = capture
            .read_cancellable(0, &limits, &mut cache, &cancel)
            .expect_err("a cancelled cached read must abort");
        assert!(matches!(error, RifError::Cancelled));
        assert!(
            cache.is_empty(),
            "a cancelled read must not populate the cache"
        );
    }

    #[test]
    fn read_cached_hits_reuse_the_same_arc() {
        let source = wrap(&[encode_part(b"label", b"payload\n")]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let mut cache = PartCache::default();
        let cancel = Cancel::new();

        assert!(cache.is_empty());
        let first = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        assert_eq!(first.text, "payload\n");
        assert_eq!(cache.len(), 1);

        let second = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "a hit must reuse the same text"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_evicts_the_least_recently_used_entry() {
        let source = wrap(&[
            encode_part(b"one", b"first"),
            encode_part(b"two", b"second"),
        ]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let cancel = Cancel::new();
        let mut cache = PartCache::new(1, 1 << 20);

        let first = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        let _ = capture
            .read_cached(1, &limits, &mut cache, &cancel)
            .unwrap();
        assert_eq!(cache.len(), 1, "only the entry bound is retained");

        let reloaded = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        assert!(
            !Arc::ptr_eq(&first, &reloaded),
            "the evicted part must be expanded again"
        );
    }

    #[test]
    fn cache_skips_text_larger_than_its_byte_budget() {
        let source = wrap(&[encode_part(b"label", b"payload\n")]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let mut cache = PartCache::new(8, 4);
        let text = capture
            .read_cached(0, &limits, &mut cache, &Cancel::new())
            .unwrap();
        assert_eq!(text.text, "payload\n");
        assert!(cache.is_empty(), "an over-budget text must not be cached");
        assert_eq!(cache.bytes(), 0);
    }

    #[test]
    fn a_cache_hit_still_honours_tighter_limits() {
        let source = wrap(&[encode_part(b"label", b"0123456789")]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let mut cache = PartCache::default();
        let cancel = Cancel::new();
        let text = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        assert_eq!(text.text.len(), 10);

        let tight = CaptureLimits {
            max_part_bytes: 4,
            ..limits
        };
        let error = capture
            .read_cached(0, &tight, &mut cache, &cancel)
            .unwrap_err();
        assert!(matches!(error, RifError::PayloadAboveLimit { limit: 4 }));
    }

    #[test]
    fn a_cache_hit_uses_the_plain_length_for_lossy_text() {
        // One invalid byte decodes to a three-byte replacement character, so the
        // text length overstates the decompressed size. A two-byte budget must
        // still accept the hit because the decompressed payload was one byte.
        let source = wrap(&[encode_part(b"label", b"\xff")]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).unwrap();
        let mut cache = PartCache::default();
        let cancel = Cancel::new();
        let first = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        assert!(first.lossy);
        assert_eq!(
            first.text.len(),
            3,
            "the replacement character is three bytes"
        );

        let tight = CaptureLimits {
            max_part_bytes: 2,
            ..limits
        };
        let hit = capture.read_cached(0, &tight, &mut cache, &cancel).unwrap();
        assert!(Arc::ptr_eq(&first, &hit));

        let too_tight = CaptureLimits {
            max_part_bytes: 0,
            ..limits
        };
        let error = capture
            .read_cached(0, &too_tight, &mut cache, &cancel)
            .unwrap_err();
        assert!(matches!(error, RifError::PayloadAboveLimit { limit: 0 }));
    }

    #[test]
    fn exact_limit_boundaries_are_inclusive() {
        let payload = b"hello boundary\n";
        let encoded = encode_part(b"a_label_long_enough_to_outgrow_the_marker_line", payload);
        let source = wrap(std::slice::from_ref(&encoded));
        let defaults = CaptureLimits::default();

        // max_span_bytes: the body between the markers fits exactly.
        let mut notes = Vec::new();
        let spans = scanner::locate_parts(&source, &defaults, &mut notes).unwrap();
        let span_len = spans[0].end - spans[0].start;
        let exact_span = CaptureLimits {
            max_span_bytes: span_len,
            ..defaults
        };
        assert!(Capture::from_bytes(&source, &exact_span).is_ok());
        let tight_span = CaptureLimits {
            max_span_bytes: span_len - 1,
            ..defaults
        };
        assert!(matches!(
            Capture::from_bytes(&source, &tight_span).unwrap_err(),
            RifError::PartSpanTooLarge { limit, .. } if limit == span_len - 1
        ));

        // max_line_bytes: the single body line fits exactly.
        let line_len = encoded.len();
        let exact_line = CaptureLimits {
            max_line_bytes: line_len,
            ..defaults
        };
        assert!(Capture::from_bytes(&source, &exact_line).is_ok());
        let tight_line = CaptureLimits {
            max_line_bytes: line_len - 1,
            ..defaults
        };
        assert!(matches!(
            Capture::from_bytes(&source, &tight_line).unwrap_err(),
            RifError::LineTooLong { limit, .. } if limit == line_len - 1
        ));

        // max_parts: the part count fits exactly.
        let two = wrap(&[encode_part(b"one", b"1"), encode_part(b"two", b"2")]);
        let exact_parts = CaptureLimits {
            max_parts: 2,
            ..defaults
        };
        assert!(Capture::from_bytes(&two, &exact_parts).is_ok());
        let tight_parts = CaptureLimits {
            max_parts: 1,
            ..defaults
        };
        assert!(matches!(
            Capture::from_bytes(&two, &tight_parts).unwrap_err(),
            RifError::TooManyParts { found: 2, limit: 1 }
        ));

        // max_part_bytes: the decompressed length fits exactly.
        let capture = Capture::from_bytes(&source, &defaults).unwrap();
        let decompressed = capture.read_bytes(0, &defaults, &Cancel::new()).unwrap();
        let size = decompressed.len();
        assert_eq!(decompressed, payload);
        let exact_part = CaptureLimits {
            max_part_bytes: size,
            ..defaults
        };
        assert!(capture.read(0, &exact_part).is_ok());
        let tight_part = CaptureLimits {
            max_part_bytes: size - 1,
            ..defaults
        };
        assert!(matches!(
            capture.read(0, &tight_part).unwrap_err(),
            RifError::PayloadAboveLimit { limit } if limit == size - 1
        ));
    }

    #[test]
    fn cache_evicts_by_bytes_preserving_mru() {
        let source = wrap(&[
            encode_part(b"one", b"a"),
            encode_part(b"two", b"b"),
            encode_part(b"three", b"c"),
        ]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).expect("fixture must index");
        let cancel = Cancel::new();
        let per = capture.read(0, &limits).unwrap().text.capacity();
        assert!(per > 0, "a tiny part must retain bytes");
        for index in 1..3 {
            assert_eq!(
                capture.read(index, &limits).unwrap().text.capacity(),
                per,
                "tiny parts must retain equally for a byte budget"
            );
        }
        let mut cache = PartCache::new(8, per * 2);
        let first = capture
            .read_cached(0, &limits, &mut cache, &cancel)
            .unwrap();
        let _ = capture
            .read_cached(1, &limits, &mut cache, &cancel)
            .unwrap();
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.bytes(), per * 2);
        let hit = cache.get(0).expect("promoted entry must hit");
        assert!(Arc::ptr_eq(&first, &hit));
        let third = capture
            .read_cached(2, &limits, &mut cache, &cancel)
            .unwrap();
        assert_eq!(third.text, "c");
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.bytes(), per * 2);
        let survivor = cache.get(0).expect("MRU must survive eviction");
        assert!(Arc::ptr_eq(&first, &survivor));
        assert!(cache.get(1).is_none(), "LRU must have been evicted");
    }

    #[test]
    fn find_first_named_returns_the_first_exact_match() {
        let source = wrap(&[
            encode_part(b"dup", b"one"),
            encode_part(b"dup", b"two"),
            encode_part(b"other", b"three"),
        ]);
        let capture =
            Capture::from_bytes(&source, &CaptureLimits::default()).expect("fixture must index");
        // Duplicates return the first index; missing labels return `None`.
        assert_eq!(capture.find_first_named("dup"), Some(0));
        assert_eq!(capture.find_first_named("other"), Some(2));
        assert_eq!(capture.find_first_named("missing"), None);
        // Case differs: the match is exact, not case-insensitive.
        assert_eq!(capture.find_first_named("DUP"), None);
        // Stays consistent with the full scan.
        for label in ["dup", "other", "missing", "DUP"] {
            assert_eq!(
                capture.find_first_named(label),
                capture.indices_named(label).first().copied(),
                "label {label:?}"
            );
        }
    }
}
