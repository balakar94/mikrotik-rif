//! Compile-time build identity: version, git commit and a short build hash.
//!
//! The Updates and About screens quote the running version and a short build
//! hash. The hash is a SHA-256 truncation of the full git commit the binary
//! was compiled from (`MIKROTIK_RIF_GIT_COMMIT`, injected by `build.rs`): it is
//! stable for a given commit, short enough to paste into a bug report, and
//! distinct from the release's own `SHA256SUMS.txt` digest, which covers the
//! installer file rather than the commit.
//!
//! When a build has no git metadata (source tarball, packaged build) the
//! commit is `unknown` and the digest still renders deterministically.

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};

/// The crate version declared in `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Full git commit this binary was compiled from, or `unknown`.
pub const GIT_COMMIT: &str = env!("MIKROTIK_RIF_GIT_COMMIT");

/// Characters kept from the commit identifier when abbreviating it.
const COMMIT_PREFIX_LEN: usize = 7;

/// Length of the user-facing build hash, in hex characters.
const BUILD_HASH_LEN: usize = 12;

/// Short form of [`GIT_COMMIT`] (at most [`COMMIT_PREFIX_LEN`] characters).
#[must_use]
pub fn commit_short() -> String {
    GIT_COMMIT.chars().take(COMMIT_PREFIX_LEN).collect()
}

/// Short SHA-256 of the full commit, e.g. `a1b2c3d4e5f6`.
///
/// This is the "build hash" shown in the interface. It is computed over the
/// commit identifier, so it is stable and reproducible for every build of the
/// same commit.
#[must_use]
pub fn build_hash_short() -> String {
    sha256_hex(GIT_COMMIT)
        .chars()
        .take(BUILD_HASH_LEN)
        .collect()
}

/// Lowercase SHA-256 hex digest of `input`.
#[must_use]
pub fn sha256_hex(input: &str) -> String {
    let digest = Sha256::digest(input.as_bytes());
    let mut out = String::with_capacity(64);
    for &byte in digest.as_slice() {
        // Writing into a `String` cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_a_known_vector() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sha256_hex("").len(), 64);
    }

    #[test]
    fn build_hash_is_a_short_lowercase_digest() {
        let hash = build_hash_short();
        assert_eq!(hash.len(), BUILD_HASH_LEN);
        assert!(
            hash.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
    }

    #[test]
    fn commit_short_is_bounded() {
        assert!(commit_short().chars().count() <= COMMIT_PREFIX_LEN);
    }
}
