//! Reader for MikroTik RouterOS `supout` capture archives.
//!
//! A capture is a text envelope: line-delimited marker pairs, each wrapping one
//! named part whose bytes are encoded with a base64-family alphabet and a
//! payload compressed as a zlib stream. This module turns that envelope into an
//! index of parts, then expands individual parts on demand into text (or, via
//! [`Capture::read_bytes`], into raw decompressed bytes).
//!
//! Indexing transcodes every part body once into a single buffer and records
//! where the compressed payload starts inside it, so opening a large capture
//! does not keep second copies of every payload. [`Cancel`] tokens let a caller
//! abandon a long scan or expansion cooperatively. [`PartCache`] avoids
//! re-expanding a part the caller has already read.
//!
//! It is deliberately UI-agnostic and free of filesystem access, so it can be
//! exercised on its own. It does not depend on any network stack, and it never
//! writes to disk.
//!
//! # Example
//!
//! ```no_run
//! use mikrotik_rif::parser::{Capture, CaptureLimits};
//!
//! let bytes = std::fs::read("supout.rif").expect("capture file");
//! let limits = CaptureLimits::default();
//! let capture = Capture::from_bytes(&bytes, &limits).expect("index the capture");
//!
//! for (index, part) in capture.parts().iter().enumerate() {
//!     println!("{index}: {} ({} compressed bytes)", part.label(), part.compressed_len());
//! }
//!
//! if let Some(first) = capture.parts().first() {
//!     let _ = first.label();
//! }
//! # let _ = capture.read(0, &limits);
//! ```

pub mod codec;
pub mod deflate;
pub mod error;
pub mod limits;
pub mod scanner;

mod capture;

#[cfg(test)]
mod roundtrip;

pub use capture::{Capture, Part, PartCache, PartText};
pub use capture::{compute_view, filter_parts, next_match};
pub use limits::{Cancel, CaptureLimits};

/// Human-readable product name.
pub const PRODUCT_NAME: &str = "MikroTik RIF Viewer";

/// Package and binary name.
pub const CODE_NAME: &str = "mikrotik-rif";
