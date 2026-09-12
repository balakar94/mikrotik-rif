//! Resource budgets applied while reading a capture.
//!
//! A support capture is attacker-influenced input once it leaves the router:
//! any field can be oversized or internally inconsistent. Every limit here has
//! a conservative default so an untrusted file cannot exhaust memory.

/// Budgets enforced by [`crate::parser::Capture::from_bytes`] and part expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLimits {
    /// Maximum number of parts accepted in a single capture.
    pub max_parts: usize,

    /// Maximum size, in bytes, of one raw part body before transcoding.
    pub max_span_bytes: usize,

    /// Maximum size, in bytes, of one decompressed part.
    pub max_part_bytes: usize,

    /// Maximum length of a single container line.
    pub max_line_bytes: usize,

    /// When `true`, structural marker problems abort instead of being noted.
    pub strict_markers: bool,

    /// When `true`, a label that is not valid UTF-8 flags the part.
    pub strict_labels: bool,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        Self {
            max_parts: 100_000,
            max_span_bytes: 128 * 1024 * 1024,
            max_part_bytes: 256 * 1024 * 1024,
            max_line_bytes: 64 * 1024 * 1024,
            strict_markers: false,
            strict_labels: false,
        }
    }
}
