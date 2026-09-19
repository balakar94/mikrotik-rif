//! Fuzz target: bounded zlib expansion (`deflate::expand`).
//!
//! A small output limit keeps decompression bombs cheap to reject while still
//! letting the fuzzer build real zlib streams. Only panic-freedom is asserted;
//! a non-zlib payload returns `RifError::Deflate` and is not a finding.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mikrotik_rif::parser::deflate;

/// Decompressed-output budget. Large enough for real fixtures, small enough
/// that the cap trips before the fuzzer spends memory on a bomb.
const EXPAND_LIMIT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    let _ = deflate::expand(data, EXPAND_LIMIT);
});
