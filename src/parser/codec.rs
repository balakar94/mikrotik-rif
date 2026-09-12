//! Symbol-to-byte transcoding for the capture envelope.
//!
//! Every part of a RouterOS support capture is wrapped in a text envelope whose
//! alphabet is base64 extended with `=` as a 65th symbol. The bit packing is
//! **not** standard base64: each group of four symbols is read as a
//! little-endian base-64 number (first symbol is the least-significant sextet)
//! and the resulting 24-bit word is emitted least-significant byte first.
//!
//! ```text
//! word = s0 | s1 << 6 | s2 << 12 | s3 << 18
//! out  = [ word & 0xff, (word >> 8) & 0xff, (word >> 16) & 0xff ]
//! ```
//!
//! Because a group always yields exactly three bytes, the decoded stream is a
//! multiple of three bytes long. The real payload is therefore followed by zero,
//! one, or two `0x00` bytes that carry no length information. Never infer the
//! payload length from the padding symbol; let the zlib stream frame itself.

use crate::parser::error::RifError;

/// Number of ASCII symbols consumed per emitted group.
pub const SYMBOLS_PER_GROUP: usize = 4;

/// Number of raw bytes produced per group.
pub const BYTES_PER_GROUP: usize = 3;

/// The 65-symbol alphabet, in numeric order. Index 64 is the padding symbol.
pub const SYMBOL_ALPHABET: &[u8; 65] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";

/// Decode the six-bit value carried by one symbol.
///
/// Padding (`=`) decodes to zero, matching the encoder, which pads the byte
/// stream with zeros and then transcodes it without a length field.
#[inline]
#[must_use]
pub fn sextet(symbol: u8) -> Option<u8> {
    match symbol {
        b'A'..=b'Z' => Some(symbol - b'A'),
        b'a'..=b'z' => Some(symbol - b'a' + 26),
        b'0'..=b'9' => Some(symbol - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        b'=' => Some(0),
        _ => None,
    }
}

/// Encode a six-bit value as its alphabet symbol.
#[inline]
#[must_use]
fn symbol(value: u32) -> u8 {
    SYMBOL_ALPHABET[(value & 0x3f) as usize]
}

/// Transcode one group of four symbols into its three raw bytes.
#[inline]
fn unpack_group(group: &[u8], base: usize) -> Result<[u8; BYTES_PER_GROUP], RifError> {
    let mut word: u32 = 0;
    for (offset, &raw) in group.iter().enumerate() {
        let value = sextet(raw).ok_or(RifError::UnknownSymbol {
            symbol: raw,
            index: base + offset,
        })?;
        word |= u32::from(value) << (6 * offset);
    }
    Ok([
        (word & 0xff) as u8,
        ((word >> 8) & 0xff) as u8,
        ((word >> 16) & 0xff) as u8,
    ])
}

/// Decode a part body made of whitespace-separated symbols into raw bytes.
///
/// ASCII whitespace is skipped, so the caller can pass the raw byte range
/// between two markers without normalizing line endings first.
///
/// # Errors
///
/// Fails when the symbol count is not a multiple of four or when a byte is not
/// part of the alphabet.
pub fn unpack(body: &[u8]) -> Result<Vec<u8>, RifError> {
    let mut symbols = body
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
        .peekable();
    let mut out = Vec::new();
    let mut base = 0usize;

    loop {
        let mut group = [0u8; SYMBOLS_PER_GROUP];
        let mut filled = 0usize;
        while filled < SYMBOLS_PER_GROUP {
            match symbols.next() {
                Some(raw) => {
                    group[filled] = raw;
                    filled += 1;
                }
                None => break,
            }
        }
        if filled == 0 {
            break;
        }
        if filled < SYMBOLS_PER_GROUP {
            return Err(RifError::SymbolCountUnaligned {
                symbols: base + filled,
                group: SYMBOLS_PER_GROUP,
            });
        }
        out.extend_from_slice(&unpack_group(&group, base)?);
        base += SYMBOLS_PER_GROUP;
    }

    Ok(out)
}

/// Encode raw bytes into the envelope alphabet.
///
/// The byte stream is zero-padded to a multiple of three before transcoding,
/// mirroring the encoder that produces real captures. Intended for fixtures and
/// round-trip tests, but public so downstream tooling can synthesise captures.
#[must_use]
pub fn pack(bytes: &[u8]) -> Vec<u8> {
    let groups = bytes.len().div_ceil(BYTES_PER_GROUP);
    let mut out = Vec::with_capacity(groups * SYMBOLS_PER_GROUP);
    for chunk in bytes.chunks(BYTES_PER_GROUP) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let word = b0 | (b1 << 8) | (b2 << 16);
        out.push(symbol(word));
        out.push(symbol(word >> 6));
        out.push(symbol(word >> 12));
        out.push(symbol(word >> 18));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sextet_is_the_inverse_of_the_alphabet() {
        for (index, &raw) in SYMBOL_ALPHABET.iter().enumerate() {
            assert_eq!(sextet(raw), Some(u8::try_from(index % 64).unwrap()));
        }
    }

    #[test]
    fn padding_decodes_to_zero() {
        assert_eq!(sextet(b'='), Some(0));
    }

    #[test]
    fn round_trip_keeps_the_payload_and_adds_zero_padding() {
        for length in 0..24usize {
            let bytes: Vec<u8> = (0..length)
                .map(|i| u8::try_from((i * 37 + 11) % 256).expect("value is below 256"))
                .collect();
            let decoded = unpack(&pack(&bytes)).expect("pack output must unpack");
            assert_eq!(&decoded[..length], &bytes[..]);
            assert!(
                decoded[length..].iter().all(|&byte| byte == 0),
                "trailing bytes must be zero padding"
            );
            assert_eq!(
                decoded.len(),
                length.div_ceil(BYTES_PER_GROUP) * BYTES_PER_GROUP
            );
        }
    }

    #[test]
    fn whitespace_between_symbols_is_ignored() {
        let packed = pack(b"routeros");
        let mut spaced = Vec::new();
        for (index, byte) in packed.iter().enumerate() {
            if index % 3 == 0 {
                spaced.push(b'\n');
            }
            spaced.push(*byte);
        }
        assert_eq!(
            unpack(&spaced).expect("spaced input must unpack"),
            unpack(&packed).unwrap()
        );
    }

    #[test]
    fn unaligned_symbol_count_is_rejected() {
        let error = unpack(b"AAA").expect_err("three symbols are not a group");
        assert!(matches!(
            error,
            RifError::SymbolCountUnaligned { symbols: 3, .. }
        ));
    }

    #[test]
    fn unknown_symbol_reports_its_offset() {
        let error = unpack(b"AA*A").expect_err("star is not in the alphabet");
        assert!(matches!(
            error,
            RifError::UnknownSymbol {
                symbol: b'*',
                index: 2
            }
        ));
    }
}
