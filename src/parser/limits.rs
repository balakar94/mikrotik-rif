//! Resource budgets applied while reading a capture.
//!
//! A support capture is attacker-influenced input once it leaves the router:
//! any field can be oversized or internally inconsistent. Every limit here has
//! a conservative default so an untrusted file cannot exhaust memory.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Cooperative cancellation flag shared by a caller and a long-running read.
///
/// The token is an owned handle around an [`AtomicBool`], so it can be cloned
/// and moved to whichever thread raises the cancellation while the worker
/// thread keeps a clone to poll. Every clone observes the same flag.
///
/// Cancellation is advisory: an operation checks the flag at coarse checkpoints
/// and may still complete if it has already passed the last one. A fresh token
/// (or `Cancel::default()`) is never cancelled, which is what the
/// non-cancellable convenience entry points use.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Create a token that is not yet cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the operation holding a clone of this token to stop.
    ///
    /// Setting the flag is idempotent; later calls are no-ops.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    /// A cloneable handle to the same flag, for handing to another thread.
    #[must_use]
    pub fn flag(&self) -> Self {
        self.clone()
    }
}

/// Budgets enforced by [`crate::parser::Capture::from_bytes`] and part expansion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureLimits {
    /// Maximum number of parts accepted in a single capture.
    pub max_parts: usize,

    /// Maximum size, in bytes, of one raw part body before transcoding.
    ///
    /// The body is the byte range between the two container markers, including
    /// any trailing newline. This bounds what the transcoder is asked to walk.
    pub max_span_bytes: usize,

    /// Maximum size, in bytes, of one decompressed part.
    ///
    /// Enforced by [`crate::parser::Capture::read`],
    /// [`crate::parser::Capture::read_bytes`] and
    /// [`crate::parser::Capture::read_cached`] while inflating a payload.
    pub max_part_bytes: usize,

    /// Maximum total transcoded bytes accepted across all parts of one capture.
    ///
    /// Indexing transcodes every part body to recover its label, so a capture
    /// made of many individually small parts could still exhaust memory. This
    /// budget caps the sum of the *transcoded envelope* bytes (label, separator
    /// and compressed payload) across every part; indexing aborts with
    /// [`crate::parser::error::RifError::BudgetExceeded`] once it would be
    /// crossed.
    ///
    /// It does **not** bound decompressed output — that is
    /// [`Self::max_part_bytes`] per part — and it does **not** bound the raw
    /// file: [`crate::parser::Capture::from_reader`] additionally refuses to
    /// buffer more raw stream bytes than this value.
    pub max_total_payload_bytes: usize,

    /// Maximum length of a single container line, excluding its line ending.
    ///
    /// # Invariant
    ///
    /// Set this to at least [`Self::max_span_bytes`]. A legitimate part body can
    /// be a single line, so a smaller line budget would reject a body that the
    /// span budget allows. The defaults satisfy the invariant; custom limits
    /// that violate it may reject otherwise valid captures. The scanner reports
    /// such a line as [`crate::parser::error::RifError::LineTooLong`] before the
    /// span budget is ever consulted.
    pub max_line_bytes: usize,

    /// When `true`, structural marker problems abort instead of being noted.
    pub strict_markers: bool,

    /// When `true`, a label that is not valid UTF-8 flags the part.
    pub strict_labels: bool,
}

impl Default for CaptureLimits {
    fn default() -> Self {
        let limits = Self {
            max_parts: 100_000,
            max_span_bytes: 128 * 1024 * 1024,
            max_part_bytes: 256 * 1024 * 1024,
            max_total_payload_bytes: 512 * 1024 * 1024,
            // Keep the line budget at least as large as the span budget: a
            // single-line part body must never be rejected by the narrower of
            // the two (see the field invariant).
            max_line_bytes: 128 * 1024 * 1024,
            strict_markers: false,
            strict_labels: false,
        };
        debug_assert!(
            limits.max_line_bytes >= limits.max_span_bytes,
            "the default line budget must cover a single-line part body"
        );
        limits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_allow_a_single_line_body_within_the_span_budget() {
        let limits = CaptureLimits::default();
        assert!(
            limits.max_line_bytes >= limits.max_span_bytes,
            "a body that fits the span budget must fit on one line"
        );
    }

    #[test]
    fn a_fresh_cancel_token_is_never_cancelled() {
        let cancel = Cancel::new();
        assert!(!cancel.is_cancelled());
        assert!(!Cancel::default().is_cancelled());
    }

    #[test]
    fn clones_share_one_flag() {
        let cancel = Cancel::new();
        let handle = cancel.flag();
        assert!(!handle.is_cancelled());
        cancel.cancel();
        assert!(handle.is_cancelled(), "clones must observe the same flag");
        assert!(cancel.is_cancelled());
    }
}
