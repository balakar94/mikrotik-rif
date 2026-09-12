//! Failure modes surfaced while reading a capture.

use thiserror::Error;

/// Everything that can go wrong while locating or expanding capture parts.
///
/// Container-level problems (an unterminated part, a broken limit) abort the
/// whole read. Per-part problems are downgraded to [`crate::parser::Part`] faults so a
/// single damaged entry never hides the rest of the capture.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RifError {
    /// An opening marker was seen but the matching closing marker never came.
    #[error("part marker at byte {offset} was opened but never closed")]
    UnterminatedPart {
        /// Byte offset of the unmatched opening marker.
        offset: usize,
    },

    /// A second opening marker appeared before the first part was closed.
    #[error("nested opening marker at byte {offset}")]
    NestedOpen {
        /// Byte offset of the offending marker.
        offset: usize,
    },

    /// A closing marker appeared outside of any part.
    #[error("closing marker at byte {offset} has no matching opening marker")]
    StrayClose {
        /// Byte offset of the offending marker.
        offset: usize,
    },

    /// A single line grew past the configured safety limit.
    #[error("line at byte {offset} is longer than the {limit}-byte line limit")]
    LineTooLong {
        /// Byte offset at which the line starts.
        offset: usize,
        /// Configured limit.
        limit: usize,
    },

    /// The capture declared more parts than the configured budget allows.
    #[error("capture holds {found} parts, above the {limit} budget")]
    TooManyParts {
        /// Number of parts found.
        found: usize,
        /// Configured limit.
        limit: usize,
    },

    /// One part body is larger than the configured budget.
    #[error("part body at byte {offset} is larger than the {limit}-byte limit")]
    PartSpanTooLarge {
        /// Byte offset at which the part body starts.
        offset: usize,
        /// Configured limit.
        limit: usize,
    },

    /// The symbol stream is not a whole number of four-symbol groups.
    #[error("symbol count {symbols} is not a multiple of {group}")]
    SymbolCountUnaligned {
        /// Number of symbols seen.
        symbols: usize,
        /// Group size (always four, kept in the error for readability).
        group: usize,
    },

    /// A symbol outside the capture alphabet was found.
    #[error("byte 0x{symbol:02x} at symbol {index} is not part of the capture alphabet")]
    UnknownSymbol {
        /// The offending byte.
        symbol: u8,
        /// Position of the byte within the symbol stream.
        index: usize,
    },

    /// The label field was not followed by a NUL separator.
    #[error("part label is not terminated by a NUL byte")]
    MissingLabelSeparator,

    /// The part declared a label but carried no compressed payload.
    #[error("part {position} carries no payload")]
    EmptyPayload {
        /// Position of the part within the capture.
        position: usize,
    },

    /// A part could not be indexed; the reason is kept for diagnostics.
    #[error("part {label:?} could not be indexed: {reason}")]
    PartUnreadable {
        /// Placeholder label shown in listings.
        label: String,
        /// Human-readable reason.
        reason: String,
    },

    /// The caller asked for a part index that does not exist.
    #[error("part index {index} is out of range")]
    NoSuchPart {
        /// Requested index.
        index: usize,
    },

    /// The caller tried to expand a part that was already flagged as unreadable.
    #[error("part {label:?} is flagged as unreadable and cannot be expanded")]
    PartFlagged {
        /// Label of the flagged part.
        label: String,
    },

    /// Decompression would exceed the configured output budget.
    #[error("payload grew beyond the {limit}-byte limit")]
    PayloadAboveLimit {
        /// Configured limit.
        limit: usize,
    },

    /// The payload is not a valid zlib stream.
    #[error("payload is not a valid zlib stream")]
    Deflate {
        /// Underlying zlib/IO failure.
        #[source]
        source: std::io::Error,
    },
}
