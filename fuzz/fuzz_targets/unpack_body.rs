//! Fuzz target: envelope transcoding (`codec`).
//!
//! Both the budgeted and the unbounded decode are exercised. The budget comes
//! from the input so the fuzzer explores the exact-overflow boundary around
//! `UnpackCapped`'s per-group check.
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use mikrotik_rif::parser::codec;

#[derive(Debug, Arbitrary)]
struct Input {
    /// Decoded-byte budget handed to `unpack_capped`.
    budget: u16,
    /// Raw symbol body, including any whitespace or out-of-alphabet bytes.
    body: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let _ = codec::unpack_capped(&input.body, usize::from(input.budget));
    let _ = codec::unpack(&input.body);
});
