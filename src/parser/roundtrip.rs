//! End-to-end checks against synthetic captures built by the tests themselves.
//!
//! No real `supout.rif` file is committed to this repository: captures can carry
//! sensitive router configuration. Fixtures are generated in memory with the
//! inverse of the reader, which also gives the transcoder a genuine round trip.

use std::io::Write;

use crate::parser::capture::Part;
use crate::parser::error::RifError;
use crate::parser::scanner::{CLOSE_MARKER, OPEN_MARKER};
use crate::parser::{Capture, CaptureLimits, codec};
use flate2::Compression;
use flate2::write::ZlibEncoder;

fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

/// Encode one part the way the router would: label, NUL, zlib payload.
fn encode_part(label: &str, body: &[u8]) -> Vec<u8> {
    let mut plain = label.as_bytes().to_vec();
    plain.push(0);
    plain.extend_from_slice(&deflate(body));
    codec::pack(&plain)
}

/// Wrap encoded parts in the container markers.
fn build_capture(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut source = Vec::new();
    for (label, body) in parts {
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(&encode_part(label, body));
        source.push(b'\n');
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
    }
    source
}

#[test]
fn reads_back_every_part_in_order() {
    let source = build_capture(&[
        ("/system/resource", b"uptime: 1d2h3m\nversion: 7.16.2\n"),
        ("/interface/print", b"name=ether1 running=yes\n"),
        ("log", b"jan/02 03:04:05 system,info rebooted\n"),
    ]);

    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("capture must index");
    assert_eq!(capture.len(), 3);
    assert!(capture.notes().is_empty());

    let labels: Vec<&str> = capture.parts().iter().map(Part::label).collect();
    assert_eq!(
        labels,
        ["/system/resource", "/interface/print", "log"],
        "parts must keep file order"
    );

    let body = capture.read(1, &limits).expect("second part must expand");
    assert_eq!(body.text, "name=ether1 running=yes\n");
    assert!(!body.lossy);
}

#[test]
fn duplicate_labels_are_kept_and_numbered() {
    let source = build_capture(&[
        ("export", b"first\n"),
        ("other", b"middle\n"),
        ("export", b"second\n"),
        ("export", b"third\n"),
    ]);

    let capture = Capture::from_bytes(&source, &CaptureLimits::default()).unwrap();
    let ordinals: Vec<(&str, usize)> = capture
        .parts()
        .iter()
        .map(|part| (part.label(), part.ordinal()))
        .collect();

    assert_eq!(
        ordinals,
        [("export", 0), ("other", 0), ("export", 1), ("export", 2)]
    );
    assert_eq!(capture.indices_named("export"), [0, 2, 3]);
    assert_eq!(capture.indices_named("other"), [1]);
    assert!(capture.indices_named("missing").is_empty());
}

#[test]
fn a_damaged_part_does_not_hide_the_rest() {
    let mut source = Vec::new();
    // Healthy part.
    source.extend_from_slice(OPEN_MARKER);
    source.push(b'\n');
    source.extend_from_slice(&encode_part("good", b"payload\n"));
    source.push(b'\n');
    source.extend_from_slice(CLOSE_MARKER);
    source.push(b'\n');
    // Part with a symbol outside the alphabet.
    source.extend_from_slice(OPEN_MARKER);
    source.push(b'\n');
    source.extend_from_slice(b"AAAA*A\n");
    source.extend_from_slice(CLOSE_MARKER);
    source.push(b'\n');
    // Another healthy part.
    source.extend_from_slice(OPEN_MARKER);
    source.push(b'\n');
    source.extend_from_slice(&encode_part("also-good", b"more\n"));
    source.push(b'\n');
    source.extend_from_slice(CLOSE_MARKER);
    source.push(b'\n');

    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("container stays readable");
    assert_eq!(capture.len(), 3);
    assert!(capture.parts()[0].is_readable());
    assert!(!capture.parts()[1].is_readable());
    assert!(capture.parts()[1].fault().is_some());
    assert!(capture.parts()[2].is_readable());

    let surfaced = capture
        .read(1, &limits)
        .expect_err("flagged part must not expand");
    assert!(matches!(surfaced, RifError::PartFlagged { .. }));
    assert_eq!(capture.read(2, &limits).unwrap().text, "more\n");
}

#[test]
fn unterminated_part_aborts_indexing() {
    let mut source = Vec::new();
    source.extend_from_slice(OPEN_MARKER);
    source.push(b'\n');
    source.extend_from_slice(b"AAAA\n");

    let error =
        Capture::from_bytes(&source, &CaptureLimits::default()).expect_err("open part must abort");
    assert!(matches!(error, RifError::UnterminatedPart { .. }));
}

#[test]
fn expansion_respects_the_output_budget() {
    let big = vec![b'x'; 8192];
    let source = build_capture(&[("big", &big)]);
    let capture = Capture::from_bytes(&source, &CaptureLimits::default()).unwrap();

    let limits = CaptureLimits {
        max_part_bytes: 1024,
        ..CaptureLimits::default()
    };
    let error = capture.read(0, &limits).expect_err("limit must trip");
    assert!(matches!(error, RifError::PayloadAboveLimit { limit: 1024 }));
}

#[test]
fn part_count_budget_is_enforced() {
    let source = build_capture(&[("a", b"1"), ("b", b"2"), ("c", b"3")]);
    let limits = CaptureLimits {
        max_parts: 2,
        ..CaptureLimits::default()
    };
    let error = Capture::from_bytes(&source, &limits).expect_err("budget must trip");
    assert!(matches!(
        error,
        RifError::TooManyParts { found: 3, limit: 2 }
    ));
}
