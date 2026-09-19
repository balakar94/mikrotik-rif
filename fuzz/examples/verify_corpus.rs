//! Smoke-test the committed seed corpus against the real parser.
//!
//! ```text
//! cargo run --manifest-path fuzz/Cargo.toml --example verify_corpus
//! ```
//!
//! This is NOT a fuzz target: it guards `gen_corpus.py` against drifting from
//! the Rust transcode by asserting the intended outcome of every committed
//! seed. Run it whenever the corpus generator or the envelope format changes.
#![forbid(unsafe_code)]

use std::fs;
use std::path::PathBuf;

use mikrotik_rif::parser::error::RifError;
use mikrotik_rif::parser::{Cancel, Capture, CaptureLimits};

fn corpus(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("corpus/parse_capture")
        .join(name);
    fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn main() {
    let limits = CaptureLimits::default();

    let valid =
        Capture::from_bytes(&corpus("valid_one_part"), &limits).expect("valid seed indexes");
    assert_eq!(valid.len(), 1);
    assert!(valid.parts()[0].is_readable());
    assert_eq!(valid.read(0, &limits).unwrap().text, "hello\n");

    let empty = Capture::from_bytes(&corpus("empty"), &limits).expect("empty seed indexes");
    assert!(empty.is_empty());

    let stray = Capture::from_bytes(&corpus("stray_close"), &limits).expect("stray close indexes");
    assert!(stray.is_empty());
    assert!(!stray.notes().is_empty(), "stray close must be noted");

    let error = Capture::from_bytes(&corpus("unterminated"), &limits).unwrap_err();
    assert!(
        matches!(error, RifError::UnterminatedPart { .. }),
        "unexpected error: {error:?}"
    );

    let nested = Capture::from_bytes(&corpus("nested_open"), &limits).expect("nested indexes");
    assert_eq!(nested.len(), 1, "the first part must be dropped");
    assert_eq!(nested.read(0, &limits).unwrap().text, "two");

    let damaged = Capture::from_bytes(&corpus("damaged_body"), &limits).expect("damaged indexes");
    assert_eq!(damaged.len(), 1);
    assert!(damaged.parts()[0].is_readable(), "the label still indexes");
    assert!(
        damaged.read_bytes(0, &limits, &Cancel::default()).is_err(),
        "the non-zlib payload must fail to expand"
    );

    let lossy = Capture::from_bytes(&corpus("lossy_label"), &limits).expect("lossy indexes");
    assert!(lossy.parts()[0].is_lossy_label());
    assert_eq!(lossy.read(0, &limits).unwrap().text, "x");

    println!("all 7 seeds behave as documented");
}
