//! Fuzz target: container scanning (`scanner::locate_parts`).
//!
//! The scan produces either spans, notes (`NestedOpen`, `StrayClose`,
//! `TrailingText`) or a limit error. Only panic-freedom is asserted; the notes
//! vector is intentionally fresh per iteration and its contents are not checked.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mikrotik_rif::parser::CaptureLimits;
use mikrotik_rif::parser::scanner::{self, ContainerNote};

/// Small budgets, identical in spirit to the other targets: a hostile file
/// reaches the limit checks quickly.
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
    let mut notes: Vec<ContainerNote> = Vec::new();
    let _ = scanner::locate_parts(data, &limits, &mut notes);
});
