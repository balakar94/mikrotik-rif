//! Bounded zlib expansion for part payloads.
//!
//! Payloads are zlib streams (RFC 1950) with a small amount of zero padding
//! after the stream end. The expander stops at the stream end marker and never
//! trusts the padding for framing.

use std::io::{ErrorKind, Read};

use flate2::read::ZlibDecoder;

use crate::parser::error::RifError;

/// Read chunk size. Small enough to stay under any UI latency budget, large
/// enough to keep syscall-free in-memory decoding cheap.
const READ_CHUNK: usize = 32 * 1024;

/// Expand a zlib payload while keeping the output under `limit` bytes.
///
/// # Errors
///
/// Returns [`RifError::PayloadAboveLimit`] as soon as the output would cross
/// `limit`, and [`RifError::Deflate`] when the stream is truncated or not zlib.
pub fn expand(payload: &[u8], limit: usize) -> Result<Vec<u8>, RifError> {
    let mut decoder = ZlibDecoder::new(payload);
    let mut out: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; READ_CHUNK.min(limit.max(1))];

    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if out.len() + read > limit {
                    return Err(RifError::PayloadAboveLimit { limit });
                }
                out.extend_from_slice(&chunk[..read]);
            }
            Err(source)
                if matches!(
                    source.kind(),
                    ErrorKind::UnexpectedEof | ErrorKind::InvalidData
                ) =>
            {
                return Err(RifError::Deflate { source });
            }
            Err(source) => return Err(RifError::Deflate { source }),
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn expands_a_zlib_stream() {
        let original = b"routeros diagnostic output".repeat(64);
        let expanded = expand(&deflate(&original), original.len() + 16).unwrap();
        assert_eq!(expanded, original);
    }

    #[test]
    fn tolerates_zero_padding_after_the_stream() {
        let original = b"counters".to_vec();
        let mut payload = deflate(&original);
        payload.extend_from_slice(&[0, 0, 0]);
        assert_eq!(expand(&payload, 1024).unwrap(), original);
    }

    #[test]
    fn refuses_to_grow_past_the_limit() {
        let original = vec![b'x'; 4096];
        let error = expand(&deflate(&original), 1024).expect_err("limit must trip");
        assert!(matches!(error, RifError::PayloadAboveLimit { limit: 1024 }));
    }

    #[test]
    fn rejects_a_non_zlib_payload() {
        let error = expand(b"not a stream at all", 1024).expect_err("garbage must fail");
        assert!(matches!(error, RifError::Deflate { .. }));
    }
}
