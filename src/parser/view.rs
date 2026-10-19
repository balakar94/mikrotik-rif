//! Line index for the reading surface, with a bounded footprint.
//!
//! The viewer scrolls a virtualised list of rows, so it needs three things
//! about an expanded module: how many lines it has, where each line starts, and
//! how wide the widest one is. A plain `Vec<usize>` of line starts answers that
//! directly, but its size is proportional to the *decompressed* text: a part
//! made only of newlines reaches `max_part_bytes` (256 MiB) while compressing to
//! a few hundred KiB, and indexing it would allocate ~2 GiB of offsets — an
//! infallible allocation failure that aborts the process before the panic hook
//! can run.
//!
//! [`LineIndex`] keeps one byte offset every [`CHECKPOINT_STRIDE`] lines and
//! scans forward (at most `CHECKPOINT_STRIDE` lines) to resolve a row, so the
//! index is bounded by `text.len() / CHECKPOINT_STRIDE` entries — about 2 MiB
//! for the worst case above, and a rounding error for ordinary text.

use std::ops::Range;

/// Lines between two stored byte offsets.
///
/// A row lookup rescans at most this many lines, so the value trades index
/// memory (`8` bytes per stride) against lookup work. `1024` keeps the worst
/// case index at ~0.8 % of the text while leaving a scan of a few KiB.
const CHECKPOINT_STRIDE: usize = 1024;

/// Line starts (every [`CHECKPOINT_STRIDE`] lines) plus the line count and the
/// widest line in characters.
///
/// Built in a single pass over `text`. Line endings are excluded from the
/// measured width, matching the width the reading surface reserves for its
/// horizontal scroll area.
#[derive(Debug)]
pub struct LineIndex {
    /// Byte offset of line `row` for every `row` that is a multiple of
    /// [`CHECKPOINT_STRIDE`]. Always starts with `0`.
    checkpoints: Vec<usize>,
    lines: usize,
    max_line_chars: usize,
}

impl LineIndex {
    /// Index `text`.
    ///
    /// Every text has at least one line: an empty text is one empty line, and a
    /// text ending in `\n` has a final empty line, matching what a line-oriented
    /// reader shows.
    #[must_use]
    pub fn new(text: &str) -> Self {
        let mut checkpoints = vec![0];
        let mut lines = 1_usize;
        let mut max_line_chars = 0_usize;
        let mut line_start = 0_usize;

        for (offset, byte) in text.bytes().enumerate() {
            if byte != b'\n' {
                continue;
            }
            max_line_chars = max_line_chars.max(measure_line(&text[line_start..offset]));
            // The line that starts right after this `\n` is line `lines`; keep
            // its offset when it lands on a stride boundary.
            if lines.is_multiple_of(CHECKPOINT_STRIDE) {
                checkpoints.push(offset + 1);
            }
            lines += 1;
            line_start = offset + 1;
        }

        // Trailing line: either the text before the first `\n` or the tail
        // after the last one, possibly empty.
        max_line_chars = max_line_chars.max(measure_line(&text[line_start..]));

        // Growth doubles the buffer, so the raw vector can hold twice the
        // offsets it needs; shrink it so `retained_bytes` reports the real
        // footprint the cache budget accounts for.
        checkpoints.shrink_to_fit();

        Self {
            checkpoints,
            lines,
            max_line_chars,
        }
    }

    /// Number of lines. Never zero.
    #[must_use]
    pub const fn line_count(&self) -> usize {
        self.lines
    }

    /// Widest line in characters, ignoring the line ending.
    #[must_use]
    pub const fn max_line_chars(&self) -> usize {
        self.max_line_chars
    }

    /// Byte range of `row`, including its line ending when it has one.
    ///
    /// # Panics
    ///
    /// Panics when `row` is out of bounds, like indexing a slice would.
    #[must_use]
    pub fn line_range(&self, text: &str, row: usize) -> Range<usize> {
        assert!(row < self.lines, "row {row} is outside the index");
        let mut start = self.checkpoints[row / CHECKPOINT_STRIDE];
        for _ in 0..row % CHECKPOINT_STRIDE {
            start = next_line_start(text, start).expect("row is inside the index");
        }
        let end = next_line_start(text, start).unwrap_or(text.len());
        start..end
    }

    /// Every line in order, each with its row number and its line ending.
    #[must_use]
    pub fn lines<'a>(&'a self, text: &'a str) -> Lines<'a> {
        Lines {
            text,
            offset: 0,
            row: 0,
            total: self.lines,
        }
    }

    /// Heap bytes retained by the index.
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.checkpoints.capacity() * size_of::<usize>()
    }
}

/// Iterator over the lines of an indexed text, in row order.
#[derive(Debug)]
pub struct Lines<'a> {
    text: &'a str,
    offset: usize,
    row: usize,
    total: usize,
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.row >= self.total {
            return None;
        }
        let start = self.offset;
        let end = next_line_start(self.text, start).unwrap_or(self.text.len());
        self.offset = end;
        let row = self.row;
        self.row += 1;
        Some((row, &self.text[start..end]))
    }
}

/// Byte offset of the line following `from`, or `None` on the last line.
fn next_line_start(text: &str, from: usize) -> Option<usize> {
    text[from..].find('\n').map(|offset| from + offset + 1)
}

/// Characters in `raw`, minus a trailing carriage return.
fn measure_line(raw: &str) -> usize {
    let line = raw.strip_suffix('\r').unwrap_or(raw);
    line.chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Line starts and the widest line, computed the obvious and unbounded way.
    fn naive(text: &str) -> (Vec<usize>, usize) {
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
            longest = longest.max(measure_line(line));
        }
        (starts, longest)
    }

    /// Every row of `text` must line up with the naive index.
    fn assert_matches_naive(text: &str) {
        let (starts, longest) = naive(text);
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), starts.len(), "line count");
        assert_eq!(index.max_line_chars(), longest, "widest line");
        for (row, &start) in starts.iter().enumerate() {
            let end = starts.get(row + 1).copied().unwrap_or(text.len());
            assert_eq!(index.line_range(text, row), start..end, "row {row}");
        }
        let collected: Vec<(usize, &str)> = index.lines(text).collect();
        assert_eq!(collected.len(), starts.len(), "iterated lines");
        for (row, line) in collected {
            let end = starts.get(row + 1).copied().unwrap_or(text.len());
            assert_eq!(line, &text[starts[row]..end], "iterated row {row}");
        }
    }

    #[test]
    fn an_empty_text_is_one_empty_line() {
        let index = LineIndex::new("");
        assert_eq!(index.line_count(), 1);
        assert_eq!(index.max_line_chars(), 0);
        assert_eq!(index.line_range("", 0), 0..0);
    }

    #[test]
    fn line_ends_do_not_count_towards_the_width() {
        let text = "a\r\nlonger line\nx";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 3);
        assert_eq!(index.max_line_chars(), 11);
        assert_eq!(index.line_range(text, 1), 3..15);
    }

    #[test]
    fn a_trailing_newline_adds_an_empty_line() {
        let text = "a\nb\n";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 3);
        assert_eq!(index.line_range(text, 2), 4..4);
    }

    #[test]
    fn every_row_is_reachable_across_checkpoint_boundaries() {
        let mut text = String::new();
        for row in 0..(CHECKPOINT_STRIDE * 3 + 7) {
            text.push_str(&"x".repeat(row % 17));
            text.push('\n');
        }
        assert_matches_naive(&text);
    }

    #[test]
    fn multibyte_lines_are_sliced_on_character_boundaries() {
        let text = "ñ\nemoji 🚀\r\nラベル\n\n末";
        assert_matches_naive(text);
    }

    #[test]
    fn a_module_of_newlines_stays_within_a_bounded_index() {
        // The pathological input from the security review: 1 Mi lines (2 MiB of
        // newlines) would be 8 MiB of offsets as a plain `Vec<usize>`; the
        // checkpointed index must stay within a stride-proportional bound.
        let text = "\n".repeat(1_000_000);
        let index = LineIndex::new(&text);
        assert_eq!(index.line_count(), 1_000_001);
        assert_eq!(index.max_line_chars(), 0);
        let ceiling = (index.line_count() / CHECKPOINT_STRIDE + 2) * size_of::<usize>();
        assert!(
            index.retained_bytes() <= ceiling,
            "index retained {} bytes, ceiling {ceiling}",
            index.retained_bytes()
        );
        // The last row is still addressable in the pathological case.
        assert_eq!(index.line_range(&text, 1_000_000), 1_000_000..1_000_000);
    }

    #[test]
    fn carriage_returns_are_measured_and_kept_in_the_range() {
        // `\r` alone (a classic-Mac line break) is not a line break here: the
        // capture format is LF-delimited, and the viewer keeps the byte so the
        // text round-trips.
        let text = "a\rb\nc";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 2);
        assert_eq!(index.max_line_chars(), 3);
        assert_eq!(index.line_range(text, 0), 0..4);
    }
}
