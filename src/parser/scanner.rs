//! Locates part bodies between the two container markers.
//!
//! The container is line oriented. Markers must sit alone on a line, but the
//! scanner tolerates surrounding spaces and both LF and CRLF endings. Text that
//! sits outside a marker pair is discarded, which is what real-world captures
//! expect: a damaged part must not leak into the next one.

use crate::parser::error::RifError;
use crate::parser::limits::CaptureLimits;

/// Opens a part body.
pub const OPEN_MARKER: &[u8] = b"--BEGIN ROUTEROS SUPOUT SECTION";

/// Closes a part body.
pub const CLOSE_MARKER: &[u8] = b"--END ROUTEROS SUPOUT SECTION";

/// Maximum structural notes retained per scan.
///
/// A hostile file could otherwise pile up one note per line and exhaust
/// memory through the notes vector alone. Oddities past this cap are counted
/// but dropped.
const MAX_NOTES: usize = 1000;

/// Record a structural oddity, keeping at most [`MAX_NOTES`] entries.
///
/// `seen` counts every oddity — including the dropped ones — independently of
/// how many entries the caller pre-loaded into `notes`, so the cap stays exact
/// even for a reused buffer.
#[inline]
fn push_note(notes: &mut Vec<ContainerNote>, seen: &mut usize, note: ContainerNote) {
    *seen += 1;
    if notes.len() < MAX_NOTES {
        notes.push(note);
    }
}

/// Byte range of one part body inside the capture buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartSpan {
    /// First byte of the body.
    pub start: usize,
    /// One past the last byte of the body.
    pub end: usize,
}

/// A structural oddity that did not abort the read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContainerNote {
    /// A new part opened before the previous one closed; the previous was dropped.
    NestedOpen {
        /// Byte offset of the second opening marker.
        offset: usize,
    },
    /// A closing marker appeared outside any part.
    StrayClose {
        /// Byte offset of the stray marker.
        offset: usize,
    },
    /// Non-whitespace bytes followed the last part.
    TrailingText {
        /// Byte offset at which the trailing bytes start.
        offset: usize,
    },
}

/// Trim ASCII whitespace from both ends of a byte slice.
#[inline]
fn trim_ascii(mut line: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = line.split_first() {
        if first.is_ascii_whitespace() {
            line = rest;
        } else {
            break;
        }
    }
    while let Some((&last, rest)) = line.split_last() {
        if last.is_ascii_whitespace() {
            line = rest;
        } else {
            break;
        }
    }
    line
}

/// Return the line starting at `cursor` and the offset of its terminating `\n`.
///
/// The returned line never includes the newline or a trailing carriage return.
#[inline]
fn read_line(source: &[u8], cursor: usize) -> (&[u8], usize) {
    let end = source[cursor..]
        .iter()
        .position(|&byte| byte == b'\n')
        .map_or(source.len(), |offset| cursor + offset);
    let mut line = &source[cursor..end];
    if line.last() == Some(&b'\r') {
        line = &line[..line.len() - 1];
    }
    (line, end)
}

/// Walk the container and record every part body.
///
/// Structural problems become entries in `notes`; the read only fails when a
/// limit is exceeded or when strict marker handling is requested.
///
/// # Errors
///
/// Returns [`RifError::UnterminatedPart`] when the last part is never closed,
/// [`RifError::LineTooLong`] when a line exceeds its budget, and the marker
/// errors when [`CaptureLimits::strict_markers`] is set.
pub fn locate_parts(
    source: &[u8],
    limits: &CaptureLimits,
    notes: &mut Vec<ContainerNote>,
) -> Result<Vec<PartSpan>, RifError> {
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    let mut open: Option<usize> = None;
    let mut last_close_line_end: Option<usize> = None;
    let mut notes_seen = 0usize;

    while cursor < source.len() {
        let (line, line_end) = read_line(source, cursor);
        if line.len() > limits.max_line_bytes {
            return Err(RifError::LineTooLong {
                offset: cursor,
                limit: limits.max_line_bytes,
            });
        }

        let content = trim_ascii(line);
        if content == OPEN_MARKER {
            if open.is_some() {
                push_note(
                    notes,
                    &mut notes_seen,
                    ContainerNote::NestedOpen { offset: cursor },
                );
                if limits.strict_markers {
                    return Err(RifError::NestedOpen { offset: cursor });
                }
            }
            open = Some((line_end + 1).min(source.len()));
        } else if content == CLOSE_MARKER {
            if let Some(start) = open.take() {
                spans.push(PartSpan { start, end: cursor });
                if spans.len() > limits.max_parts {
                    return Err(RifError::TooManyParts {
                        found: spans.len(),
                        limit: limits.max_parts,
                    });
                }
            } else {
                push_note(
                    notes,
                    &mut notes_seen,
                    ContainerNote::StrayClose { offset: cursor },
                );
                if limits.strict_markers {
                    return Err(RifError::StrayClose { offset: cursor });
                }
            }
            last_close_line_end = Some(line_end);
        }

        if line_end >= source.len() {
            break;
        }
        cursor = line_end + 1;
    }

    if let Some(start) = open {
        return Err(RifError::UnterminatedPart { offset: start });
    }

    if let Some(tail) = last_close_line_end {
        let rest = &source[(tail + 1).min(source.len())..];
        if let Some(offset) = rest.iter().position(|byte| !byte.is_ascii_whitespace()) {
            push_note(
                notes,
                &mut notes_seen,
                ContainerNote::TrailingText {
                    offset: tail + 1 + offset,
                },
            );
        }
    }

    debug_assert!(
        notes_seen >= notes.len(),
        "every stored note was counted when seen"
    );
    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(parts: &[&str]) -> Vec<u8> {
        let mut source = Vec::new();
        for part in parts {
            source.extend_from_slice(OPEN_MARKER);
            source.push(b'\n');
            source.extend_from_slice(part.as_bytes());
            source.push(b'\n');
            source.extend_from_slice(CLOSE_MARKER);
            source.push(b'\n');
        }
        source
    }

    #[test]
    fn finds_every_body_and_excludes_the_markers() {
        let source = body(&["AAAA", "BBBB"]);
        let mut notes = Vec::new();
        let spans = locate_parts(&source, &CaptureLimits::default(), &mut notes).unwrap();
        assert_eq!(spans.len(), 2);
        assert!(notes.is_empty());
        // The body range keeps the line ending; the transcoder skips whitespace.
        assert_eq!(&source[spans[0].start..spans[0].end], b"AAAA\n");
        assert_eq!(&source[spans[1].start..spans[1].end], b"BBBB\n");
    }

    #[test]
    fn crlf_and_leading_spaces_are_tolerated() {
        let source =
            b"  --BEGIN ROUTEROS SUPOUT SECTION \r\nAA\r\n--END ROUTEROS SUPOUT SECTION\r\n";
        let mut notes = Vec::new();
        let spans = locate_parts(source, &CaptureLimits::default(), &mut notes).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(&source[spans[0].start..spans[0].end], b"AA\r\n");
    }

    #[test]
    fn outer_text_is_discarded() {
        let mut source = b"junk before\n".to_vec();
        source.extend_from_slice(&body(&["AAAA"]));
        source.extend_from_slice(b"junk after\n");
        let mut notes = Vec::new();
        let spans = locate_parts(&source, &CaptureLimits::default(), &mut notes).unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!(
            notes,
            vec![ContainerNote::TrailingText {
                offset: source.len() - 11
            }]
        );
    }

    #[test]
    fn nested_open_is_noted_and_the_previous_part_is_dropped() {
        let mut source = Vec::new();
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(b"AAAA\n");
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(b"BBBB\n");
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
        let mut notes = Vec::new();
        let spans = locate_parts(&source, &CaptureLimits::default(), &mut notes).unwrap();
        assert_eq!(spans.len(), 1);
        assert!(matches!(
            notes.as_slice(),
            [ContainerNote::NestedOpen { .. }]
        ));
    }

    #[test]
    fn stray_close_is_noted() {
        let mut source = b"AAAA\n".to_vec();
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
        let mut notes = Vec::new();
        let spans = locate_parts(&source, &CaptureLimits::default(), &mut notes).unwrap();
        assert!(spans.is_empty());
        assert_eq!(notes, vec![ContainerNote::StrayClose { offset: 5 }]);
    }

    #[test]
    fn unterminated_part_is_an_error() {
        let mut source = Vec::new();
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(b"AAAA\n");
        let error = locate_parts(&source, &CaptureLimits::default(), &mut Vec::new())
            .expect_err("open part must be reported");
        assert!(matches!(error, RifError::UnterminatedPart { .. }));
    }

    #[test]
    fn part_count_budget_aborts_the_scan_early() {
        let source = body(&["AAAA", "BBBB", "CCCC"]);
        let limits = CaptureLimits {
            max_parts: 2,
            ..CaptureLimits::default()
        };
        let error = locate_parts(&source, &limits, &mut Vec::new()).expect_err("budget must trip");
        assert!(matches!(
            error,
            RifError::TooManyParts { found: 3, limit: 2 }
        ));
    }

    #[test]
    fn notes_are_capped_while_scanning_continues() {
        let mut source = Vec::new();
        for _ in 0..1500 {
            source.extend_from_slice(CLOSE_MARKER);
            source.push(b'\n');
        }
        let mut notes = Vec::new();
        let spans = locate_parts(&source, &CaptureLimits::default(), &mut notes)
            .expect("stray closes must not abort");
        assert!(spans.is_empty());
        assert_eq!(notes.len(), 1000, "notes must stop growing at the cap");
        assert!(
            notes
                .iter()
                .all(|note| matches!(note, ContainerNote::StrayClose { .. }))
        );
    }

    #[test]
    fn a_hundred_thousand_spans_trip_the_default_budget() {
        let one = body(&["AAAA"]);
        let mut source = Vec::with_capacity(one.len() * 100_001);
        for _ in 0..100_001 {
            source.extend_from_slice(&one);
        }
        let error = locate_parts(&source, &CaptureLimits::default(), &mut Vec::new())
            .expect_err("default part budget must trip");
        assert!(matches!(
            error,
            RifError::TooManyParts {
                found: 100_001,
                limit: 100_000
            }
        ));
    }

    #[test]
    fn strict_mode_escalates_notes() {
        let mut source = b"AAAA\n".to_vec();
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
        let limits = CaptureLimits {
            strict_markers: true,
            ..CaptureLimits::default()
        };
        let error =
            locate_parts(&source, &limits, &mut Vec::new()).expect_err("strict mode must abort");
        assert!(matches!(error, RifError::StrayClose { .. }));
    }
}
