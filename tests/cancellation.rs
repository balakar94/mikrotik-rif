//! Cancellation semantics for indexing and expansion.
//!
//! Cancellation is cooperative and checked only at coarse checkpoints, so these
//! tests use a token that is already raised. That makes every assertion
//! deterministic: no sleep, no thread race and no timing assumption. A test
//! that tried to cancel *during* a read would be inherently flaky.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use mikrotik_rif::parser::codec;
use mikrotik_rif::parser::error::RifError;
use mikrotik_rif::parser::scanner::{CLOSE_MARKER, OPEN_MARKER};
use mikrotik_rif::parser::{Cancel, Capture, CaptureLimits, PartCache};

/// Compress `bytes` into the zlib payload a real part carries.
fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("compress fixture bytes");
    encoder.finish().expect("finish fixture stream")
}

/// Encode one part the way the router would: label, NUL, zlib payload.
fn encode_part(label: &[u8], body: &[u8]) -> Vec<u8> {
    let mut plain = label.to_vec();
    plain.push(0);
    plain.extend_from_slice(&deflate(body));
    codec::pack(&plain)
}

/// Wrap already-encoded bodies in the marker pair.
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

/// A single-part capture with a large, poorly compressible payload.
fn large_capture() -> Vec<u8> {
    // An LCG filled buffer: large enough that an unchecked read would be
    // measurable work, but deterministic so the fixture is stable.
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut body = Vec::with_capacity(1024 * 1024);
    while body.len() < 1024 * 1024 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        body.extend_from_slice(&state.to_le_bytes());
    }
    wrap(&[encode_part(b"large", &body)])
}

fn raised_token() -> Cancel {
    let cancel = Cancel::new();
    cancel.cancel();
    cancel
}

#[test]
fn a_fresh_token_is_not_cancelled() {
    assert!(!Cancel::new().is_cancelled());
    assert!(!Cancel::default().is_cancelled());
}

#[test]
fn cancelling_is_observed_by_every_clone() {
    let cancel = Cancel::new();
    let handle = cancel.flag();
    cancel.cancel();
    assert!(cancel.is_cancelled());
    assert!(handle.is_cancelled(), "clones share the same flag");
}

#[test]
fn pre_cancelled_token_aborts_indexing() {
    let source = large_capture();
    let error =
        Capture::from_bytes_cancellable(&source, &CaptureLimits::default(), &raised_token())
            .expect_err("a raised token must abort indexing");
    assert!(matches!(error, RifError::Cancelled));
}

#[test]
fn pre_cancelled_token_aborts_read_bytes_of_a_large_part() {
    let source = large_capture();
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("fixture must index");

    let error = capture
        .read_bytes(0, &limits, &raised_token())
        .expect_err("a raised token must abort expansion");
    assert!(matches!(error, RifError::Cancelled));
    assert!(
        error.to_string().contains("cancel"),
        "the Cancelled variant must be distinguishable by its message"
    );
}

#[test]
fn pre_cancelled_token_aborts_cached_read_and_keeps_cache_empty() {
    let source = large_capture();
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("fixture must index");
    let mut cache = PartCache::default();

    let error = capture
        .read_cancellable(0, &limits, &mut cache, &raised_token())
        .expect_err("a raised token must abort a cached read");
    assert!(matches!(error, RifError::Cancelled));
    assert!(
        cache.is_empty(),
        "an aborted read must not populate the cache"
    );
    assert_eq!(cache.bytes(), 0);
}

#[test]
fn a_token_cancelled_after_indexing_aborts_expansion() {
    let source = large_capture();
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("fixture must index");

    let cancel = Cancel::new();
    assert!(!cancel.is_cancelled());
    cancel.cancel();

    let error = capture
        .read_bytes(0, &limits, &cancel)
        .expect_err("cancellation after indexing must abort expansion");
    assert!(matches!(error, RifError::Cancelled));
}

#[test]
fn a_never_cancelled_token_still_reads() {
    let source = large_capture();
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes_cancellable(&source, &limits, &Cancel::new())
        .expect("a fresh token must not abort indexing");
    let bytes = capture
        .read_bytes(0, &limits, &Cancel::new())
        .expect("a fresh token must not abort expansion");
    assert_eq!(bytes.len(), 1024 * 1024);
}
