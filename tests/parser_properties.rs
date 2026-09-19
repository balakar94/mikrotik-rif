//! Property tests for the envelope codec and the indexing entry points.
//!
//! These complement the deterministic integration tests: the codec is the one
//! component whose behaviour is defined over *all* byte strings, so a
//! property test is the right tool for its round trip, its whitespace
//! tolerance and its budget monotonicity.

use mikrotik_rif::parser::codec;
use mikrotik_rif::parser::error::RifError;
use mikrotik_rif::parser::{Capture, CaptureLimits};
use proptest::prelude::*;

/// The ASCII whitespace bytes the transcoder skips: space, tab, CR, LF and
/// form feed. Vertical tab is deliberately absent because
/// [`u8::is_ascii_whitespace`] does not treat it as whitespace.
const ASCII_WHITESPACE: &[u8] = b" \t\r\n\x0c";

proptest! {
    #[test]
    fn pack_roundtrip_preserves_prefix_and_zero_pads(
        bytes in prop::collection::vec(any::<u8>(), 0..=1024),
    ) {
        let packed = codec::pack(&bytes);
        let decoded = codec::unpack(&packed).expect("pack output must decode");

        prop_assert!(decoded.starts_with(&bytes));
        prop_assert!(
            decoded[bytes.len()..].iter().all(|&byte| byte == 0),
            "everything after the payload must be zero padding"
        );
        prop_assert_eq!(decoded.len() % 3, 0);
        prop_assert_eq!(decoded.len(), bytes.len().div_ceil(3) * 3);
    }

    #[test]
    fn whitespace_between_symbols_does_not_change_the_decode(
        bytes in prop::collection::vec(any::<u8>(), 0..=256),
    ) {
        let packed = codec::pack(&bytes);
        let mut spaced = Vec::with_capacity(packed.len() * 2 + 1);
        for (index, &symbol) in packed.iter().enumerate() {
            spaced.push(ASCII_WHITESPACE[index % ASCII_WHITESPACE.len()]);
            spaced.push(symbol);
        }
        spaced.push(b'\n');

        prop_assert_eq!(
            codec::unpack(&spaced).expect("spaced input must decode"),
            codec::unpack(&packed).expect("packed input must decode")
        );
    }

    #[test]
    fn capped_decode_is_budget_monotone(
        bytes in prop::collection::vec(any::<u8>(), 0..=64),
        first in 0usize..=256,
        second in 0usize..=256,
    ) {
        let packed = codec::pack(&bytes);
        let (lower, higher) = if first <= second {
            (first, second)
        } else {
            (second, first)
        };

        if let Ok(decoded) = codec::unpack_capped(&packed, lower) {
            let again = codec::unpack_capped(&packed, higher)
                .expect("a larger budget must also decode");
            prop_assert_eq!(decoded, again);
        }
    }

    #[test]
    fn capped_decode_refuses_over_budget_input(
        bytes in prop::collection::vec(any::<u8>(), 1..=64),
    ) {
        let packed = codec::pack(&bytes);
        let decoded = codec::unpack(&packed).expect("pack output must decode");

        let error = codec::unpack_capped(&packed, decoded.len() - 1)
            .expect_err("one byte short of the payload must be refused");
        let refused_over_budget = matches!(
            error,
            RifError::BudgetExceeded { limit } if limit == decoded.len() - 1
        );
        prop_assert!(refused_over_budget);

        let exact = codec::unpack_capped(&packed, decoded.len())
            .expect("the exact budget must decode");
        prop_assert_eq!(exact, decoded);
    }

    #[test]
    fn from_bytes_never_panics_on_arbitrary_input(
        source in prop::collection::vec(any::<u8>(), 0..=4_096),
    ) {
        // Tiny budgets keep a random input from allocating much, so this also
        // probes every early-return path without slowing the suite down.
        let limits = CaptureLimits {
            max_parts: 4,
            max_span_bytes: 256,
            max_part_bytes: 256,
            max_total_payload_bytes: 512,
            max_line_bytes: 64,
            strict_markers: false,
            strict_labels: false,
        };
        let _ = Capture::from_bytes(&source, &limits);
    }
}
