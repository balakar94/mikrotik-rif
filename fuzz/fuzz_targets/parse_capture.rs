//! Fuzz target: index an attacker-influenced capture and expand every readable
//! part.
//!
//! The assertion is deliberately weak: **no panic and no hang**. Both success
//! and every `RifError` are valid outcomes; nothing semantic is asserted. The
//! limits are tiny so the parser's budget checks trip early and the fuzzer can
//! spend its time on the error paths rather than on huge allocations.
//!
//! `max_line_bytes` must stay `>= max_span_bytes` (see `CaptureLimits`): a
//! legitimate part body can be a single line, and the defaults enforce the same
//! invariant.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mikrotik_rif::parser::{Cancel, Capture, CaptureLimits};

/// Small budgets: parts, span, decompressed part, total transcoded bytes and
/// line length are all bounded so budgets trip cheaply.
fn tiny_limits() -> CaptureLimits {
    CaptureLimits {
        max_parts: 4,
        max_span_bytes: 256,
        max_part_bytes: 256,
        max_total_payload_bytes: 1024,
        max_line_bytes: 256,
        strict_markers: false,
        strict_labels: false,
    }
}

fuzz_target!(|data: &[u8]| {
    let limits = tiny_limits();

    // A container-level failure (unterminated part, budget, line length) is a
    // normal outcome; only a panic would be a finding.
    let Ok(capture) = Capture::from_bytes(data, &limits) else {
        return;
    };

    for (index, part) in capture.parts().iter().enumerate() {
        if !part.is_readable() {
            continue;
        }
        // `read_bytes` is the primitive; `read` adds UTF-8 decoding on top.
        // A malformed zlib payload simply returns `RifError::Deflate`.
        let _ = capture.read_bytes(index, &limits, &Cancel::default());
        let _ = capture.read(index, &limits);
    }
});
