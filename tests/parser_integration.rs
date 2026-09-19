//! Integration tests for the capture parser's public surface.
//!
//! Every fixture is built in memory with the inverse of the reader: a part is
//! `label`, a NUL separator and a zlib payload, transcoded with
//! [`codec::pack`] and wrapped in the two container markers. No real capture is
//! committed to this repository, because captures can carry sensitive router
//! configuration.
//!
//! The opt-in [`reads_real_capture_corpus`] test documents the
//! `MIKROTIK_RIF_CORPUS` environment variable and is ignored unless asked for.

use std::io::{self, Read, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use mikrotik_rif::parser::codec;
use mikrotik_rif::parser::error::RifError;
use mikrotik_rif::parser::scanner::{CLOSE_MARKER, OPEN_MARKER, locate_parts};
use mikrotik_rif::parser::{Cancel, Capture, CaptureLimits, Part, PartCache};

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

/// Build a complete synthetic capture from label/body pairs.
fn build_capture(parts: &[(&[u8], &[u8])]) -> Vec<u8> {
    let encoded: Vec<Vec<u8>> = parts
        .iter()
        .map(|(label, body)| encode_part(label, body))
        .collect();
    wrap(&encoded)
}

/// The externally observable identity of every indexed part.
fn part_identity(capture: &Capture) -> Vec<(&str, usize, usize, bool)> {
    capture
        .parts()
        .iter()
        .map(|part| {
            (
                Part::label(part),
                Part::ordinal(part),
                Part::compressed_len(part),
                Part::is_readable(part),
            )
        })
        .collect()
}

/// A reader that always fails, to exercise the stream error path.
struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("synthetic read failure"))
    }
}

/// A reader that yields at most one byte per call, so a single logical read is
/// split across many short reads.
struct OneByteAtATime<'a> {
    source: &'a [u8],
    position: usize,
}

impl<'a> OneByteAtATime<'a> {
    const fn new(source: &'a [u8]) -> Self {
        Self {
            source,
            position: 0,
        }
    }
}

impl Read for OneByteAtATime<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position >= self.source.len() || buf.is_empty() {
            return Ok(0);
        }
        buf[0] = self.source[self.position];
        self.position += 1;
        Ok(1)
    }
}

#[test]
fn from_reader_maps_io_failure_to_stream_read() {
    let error = Capture::from_reader(FailingReader, &CaptureLimits::default())
        .expect_err("a failing reader must surface");
    assert!(matches!(error, RifError::StreamRead { .. }));
}

#[test]
fn one_byte_reader_indexes_identically_to_bytes() {
    let source = build_capture(&[
        (b"/system/resource", b"uptime: 1d\nversion: 7.16.2\n"),
        (b"/interface/print", b"name=ether1 running=yes\n"),
        (b"log", b"jan/02 03:04:05 system,info rebooted\n"),
    ]);
    let limits = CaptureLimits::default();

    let by_bytes = Capture::from_bytes(&source, &limits).expect("buffer must index");
    let by_reader =
        Capture::from_reader(OneByteAtATime::new(&source), &limits).expect("stream must index");

    assert_eq!(by_bytes.len(), by_reader.len());
    assert_eq!(part_identity(&by_bytes), part_identity(&by_reader));

    for position in 0..by_bytes.len() {
        assert_eq!(
            by_bytes.read(position, &limits).unwrap().text,
            by_reader.read(position, &limits).unwrap().text
        );
    }
}

#[test]
fn capture_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Capture>();
}

#[test]
fn max_parts_boundary_is_inclusive() {
    let source = build_capture(&[(b"a", b"1"), (b"b", b"2")]);
    let defaults = CaptureLimits::default();

    let exact = CaptureLimits {
        max_parts: 2,
        ..defaults
    };
    assert_eq!(
        Capture::from_bytes(&source, &exact)
            .expect("exact part budget must index")
            .len(),
        2
    );

    let tight = CaptureLimits {
        max_parts: 1,
        ..defaults
    };
    assert!(matches!(
        Capture::from_bytes(&source, &tight).unwrap_err(),
        RifError::TooManyParts { found: 2, limit: 1 }
    ));
}

#[test]
fn max_span_bytes_boundary_is_inclusive() {
    let encoded = encode_part(b"span-label", b"span body\n");
    let source = wrap(std::slice::from_ref(&encoded));
    let mut notes = Vec::new();
    let spans =
        locate_parts(&source, &CaptureLimits::default(), &mut notes).expect("fixture must locate");
    let span_len = spans[0].end - spans[0].start;
    let defaults = CaptureLimits::default();

    let exact = CaptureLimits {
        max_span_bytes: span_len,
        ..defaults
    };
    assert!(Capture::from_bytes(&source, &exact).is_ok());

    let tight = CaptureLimits {
        max_span_bytes: span_len - 1,
        ..defaults
    };
    assert!(matches!(
        Capture::from_bytes(&source, &tight).unwrap_err(),
        RifError::PartSpanTooLarge { limit, .. } if limit == span_len - 1
    ));
}

#[test]
fn max_part_bytes_boundary_is_inclusive() {
    let source = build_capture(&[(b"part", b"decompressed body\n")]);
    let defaults = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &defaults).expect("fixture must index");
    let size = capture
        .read_bytes(0, &defaults, &Cancel::new())
        .expect("fixture must expand")
        .len();

    let exact = CaptureLimits {
        max_part_bytes: size,
        ..defaults
    };
    assert!(capture.read(0, &exact).is_ok());

    let tight = CaptureLimits {
        max_part_bytes: size - 1,
        ..defaults
    };
    assert!(matches!(
        capture.read(0, &tight).unwrap_err(),
        RifError::PayloadAboveLimit { limit } if limit == size - 1
    ));
}

#[test]
fn max_total_payload_bytes_boundary_is_inclusive() {
    let first = encode_part(b"one", b"first body\n");
    let second = encode_part(b"two", b"second body\n");
    let total = codec::unpack(&first).unwrap().len() + codec::unpack(&second).unwrap().len();
    let source = wrap(&[first, second]);
    let defaults = CaptureLimits::default();

    let exact = CaptureLimits {
        max_total_payload_bytes: total,
        ..defaults
    };
    assert_eq!(
        Capture::from_bytes(&source, &exact)
            .expect("exact total budget must index")
            .len(),
        2
    );

    let tight = CaptureLimits {
        max_total_payload_bytes: total - 1,
        ..defaults
    };
    assert!(matches!(
        Capture::from_bytes(&source, &tight).unwrap_err(),
        RifError::BudgetExceeded { limit } if limit == total - 1
    ));
}

#[test]
fn max_line_bytes_boundary_is_inclusive() {
    let encoded = encode_part(b"line-label", b"single line body\n");
    let source = wrap(std::slice::from_ref(&encoded));
    let defaults = CaptureLimits::default();

    let exact = CaptureLimits {
        max_line_bytes: encoded.len(),
        ..defaults
    };
    assert!(Capture::from_bytes(&source, &exact).is_ok());

    let tight = CaptureLimits {
        max_line_bytes: encoded.len() - 1,
        ..defaults
    };
    assert!(matches!(
        Capture::from_bytes(&source, &tight).unwrap_err(),
        RifError::LineTooLong { limit, .. } if limit == encoded.len() - 1
    ));
}

#[test]
fn missing_label_separator_is_a_part_fault() {
    // A six-byte body packs to exactly six bytes and carries no NUL at all, so
    // the label terminator is missing rather than merely absent from a padded
    // tail.
    let source = wrap(&[codec::pack(b"abcdef")]);
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("container stays readable");

    assert_eq!(capture.len(), 1);
    assert!(!capture.parts()[0].is_readable());
    assert!(capture.parts()[0].fault().is_some());
    assert!(matches!(
        capture.read(0, &limits).unwrap_err(),
        RifError::PartFlagged { .. }
    ));
}

#[test]
fn empty_payload_is_a_part_fault() {
    // `ab` plus the NUL separator is exactly one three-byte group, so the part
    // ends immediately after its label.
    let source = wrap(&[codec::pack(b"ab\0")]);
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("container stays readable");

    assert_eq!(capture.len(), 1);
    assert!(!capture.parts()[0].is_readable());
    assert!(capture.parts()[0].fault().is_some());
}

#[test]
fn part_span_too_large_is_a_hard_error() {
    let source = build_capture(&[(b"label", b"body\n")]);
    let limits = CaptureLimits {
        max_span_bytes: 1,
        ..CaptureLimits::default()
    };
    assert!(matches!(
        Capture::from_bytes(&source, &limits).unwrap_err(),
        RifError::PartSpanTooLarge { limit: 1, .. }
    ));
}

#[test]
fn no_such_part_is_a_hard_error() {
    let source = build_capture(&[(b"only", b"body\n")]);
    let limits = CaptureLimits::default();
    let capture = Capture::from_bytes(&source, &limits).expect("fixture must index");
    let mut cache = PartCache::default();

    assert!(matches!(
        capture.read(5, &limits).unwrap_err(),
        RifError::NoSuchPart { index: 5 }
    ));
    assert!(matches!(
        capture.read_bytes(5, &limits, &Cancel::new()).unwrap_err(),
        RifError::NoSuchPart { index: 5 }
    ));
    assert!(matches!(
        capture
            .read_cached(5, &limits, &mut cache, &Cancel::new())
            .unwrap_err(),
        RifError::NoSuchPart { index: 5 }
    ));
    assert!(cache.is_empty());
}

#[test]
fn damaged_part_does_not_hide_healthy_neighbours() {
    let first = encode_part(b"good-a", b"first\n");
    // Raw symbol line with a byte outside the alphabet, bypassing `pack`.
    let damaged = b"AAAA*AAA".to_vec();
    let last = encode_part(b"good-b", b"second\n");
    let source = wrap(&[first, damaged, last]);
    let limits = CaptureLimits::default();

    let capture = Capture::from_bytes(&source, &limits).expect("container stays readable");
    assert_eq!(capture.len(), 3);
    assert!(capture.parts()[0].is_readable());
    assert!(!capture.parts()[1].is_readable());
    assert!(capture.parts()[1].fault().is_some());
    assert!(capture.parts()[2].is_readable());

    assert_eq!(capture.read(0, &limits).unwrap().text, "first\n");
    assert_eq!(capture.read(2, &limits).unwrap().text, "second\n");
    assert!(matches!(
        capture.read(1, &limits).unwrap_err(),
        RifError::PartFlagged { .. }
    ));
    assert_eq!(capture.indices_named("good-b"), [2]);
}

/// Opt-in smoke test over a directory of real `.rif` captures.
///
/// Set `MIKROTIK_RIF_CORPUS` to a directory of known-good RouterOS
/// `supout.rif` files to run it:
///
/// ```sh
/// MIKROTIK_RIF_CORPUS=/path/to/captures \
///     cargo test --test parser_integration -- --ignored reads_real_capture_corpus
/// ```
///
/// The test returns early (skipping) when the variable is unset or does not
/// point at a directory, and it is ignored by default so CI never needs a
/// capture. Each `.rif` file must index without a container-level error; every
/// readable part must then expand within the default limits.
#[test]
#[ignore = "needs a local capture; set MIKROTIK_RIF_CORPUS to a directory of .rif files"]
fn reads_real_capture_corpus() {
    let Ok(dir) = std::env::var("MIKROTIK_RIF_CORPUS") else {
        eprintln!("MIKROTIK_RIF_CORPUS is not set; skipping real-corpus test");
        return;
    };
    let dir = Path::new(&dir);
    if !dir.is_dir() {
        eprintln!(
            "MIKROTIK_RIF_CORPUS={} is not a directory; skipping",
            dir.display()
        );
        return;
    }

    let limits = CaptureLimits::default();
    let mut checked = 0usize;
    for entry in std::fs::read_dir(dir).expect("corpus directory must be readable") {
        let path = entry.expect("corpus entry must be readable").path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rif") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("capture must be readable");
        let capture = Capture::from_bytes(&bytes, &limits)
            .unwrap_or_else(|error| panic!("{} must index: {error}", path.display()));
        for (position, part) in capture.parts().iter().enumerate() {
            if part.is_readable() {
                capture
                    .read_bytes(position, &limits, &Cancel::new())
                    .unwrap_or_else(|error| {
                        panic!(
                            "{} part {position} ({}) must expand: {error}",
                            path.display(),
                            part.label()
                        )
                    });
            }
        }
        checked += 1;
    }
    eprintln!("verified {checked} capture(s) from {}", dir.display());
}
