//! Offline reader for MikroTik RouterOS `supout` capture archives.
//!
//! A capture is a text envelope: line-delimited marker pairs, each wrapping one
//! named part whose bytes are encoded with a base64-family alphabet and a
//! payload compressed as a zlib stream. This module turns that envelope into an
//! index of parts, then expands individual parts on demand.
//!
//! It is deliberately UI-agnostic and free of filesystem access, so it can be
//! exercised on its own. It does not depend on any network stack, and it never
//! writes to disk.
//!
//! # Example
//!
//! ```no_run
//! use crate::parser::{Capture, CaptureLimits};
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

pub use capture::{Capture, Part, PartText};
pub use limits::CaptureLimits;

/// Human-readable product name.
pub const PRODUCT_NAME: &str = "MikroTik RIF Viewer";

/// Package and binary name.
pub const CODE_NAME: &str = "mikrotik-rif";
