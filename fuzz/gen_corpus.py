#!/usr/bin/env python3
"""Generate tiny seed captures for the `parse_capture` fuzz corpus.

The parser's envelope is not standard base64, so re-implementing the transcode
in a dependency-free script is the most portable way to produce valid seeds.
The encoder below mirrors `src/parser/codec.rs::pack` exactly:

    word = b0 | b1 << 8 | b2 << 16          (little-endian 24-bit word)
    out  = [ word & 0x3f, word >> 6, word >> 12, word >> 18 ]  (alphabet)

A part body is `label || 0x00 || zlib(body)`, and the container wraps it in
`--BEGIN ROUTEROS SUPOUT SECTION` / `--END ROUTEROS SUPOUT SECTION` lines.

Run from anywhere:  python3 fuzz/gen_corpus.py

Seeds are deliberately tiny (< 1 KiB each); the fuzzer grows them with its own
mutations. Re-running the script overwrites the committed seeds byte-for-byte.
"""

from __future__ import annotations

import os
import zlib

ALPHABET = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/="
OPEN_MARKER = b"--BEGIN ROUTEROS SUPOUT SECTION"
CLOSE_MARKER = b"--END ROUTEROS SUPOUT SECTION"

CORPUS_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "corpus", "parse_capture")


def pack(raw: bytes) -> bytes:
    """Transcode raw bytes exactly like `codec::pack`."""
    out = bytearray()
    for start in range(0, len(raw), 3):
        chunk = raw[start : start + 3]
        b0 = chunk[0]
        b1 = chunk[1] if len(chunk) > 1 else 0
        b2 = chunk[2] if len(chunk) > 2 else 0
        word = b0 | (b1 << 8) | (b2 << 16)
        out.append(ALPHABET[word & 0x3F])
        out.append(ALPHABET[(word >> 6) & 0x3F])
        out.append(ALPHABET[(word >> 12) & 0x3F])
        out.append(ALPHABET[(word >> 18) & 0x3F])
    return bytes(out)


def encode_part(label: bytes, body: bytes) -> bytes:
    """Build one part body: label, NUL separator, zlib payload, transcoded."""
    return pack(label + b"\x00" + zlib.compress(body))


def wrap(parts: list[bytes]) -> bytes:
    """Wrap encoded part bodies in the line-oriented container markers."""
    source = bytearray()
    for part in parts:
        source += OPEN_MARKER + b"\n"
        source += part + b"\n"
        source += CLOSE_MARKER + b"\n"
    return bytes(source)


def seeds() -> dict[str, bytes]:
    return {
        # Baseline: one well-formed, readable part.
        "valid_one_part": wrap([encode_part(b"export", b"hello\n")]),
        # Empty file: no markers, no parts.
        "empty": b"",
        # A closing marker with no matching open: scanner note, not an error.
        "stray_close": CLOSE_MARKER + b"\n",
        # An opening marker without a close: `UnterminatedPart`.
        "unterminated": OPEN_MARKER + b"\n" + b"AAAA\n",
        # A nested opening marker: previous part is dropped, note recorded.
        "nested_open": (
            OPEN_MARKER
            + b"\n"
            + encode_part(b"first", b"one")
            + b"\n"
            + OPEN_MARKER
            + b"\n"
            + encode_part(b"second", b"two")
            + b"\n"
            + CLOSE_MARKER
            + b"\n"
        ),
        # Payload is not a zlib stream: the part indexes but `read` fails.
        "damaged_body": wrap([pack(b"label\x00" + b"not-zlib!!")]),
        # Non-UTF-8 label: exercises the lossy-label / strict-label path.
        "lossy_label": wrap([encode_part(b"fo\xff\xfe", b"x")]),
    }


def main() -> None:
    os.makedirs(CORPUS_DIR, exist_ok=True)
    total = 0
    for name, blob in sorted(seeds().items()):
        # Smoke-check our own encoder round-trips before writing anything.
        if blob:
            assert len(blob) < 1024, f"{name} seed is not tiny ({len(blob)} bytes)"
        path = os.path.join(CORPUS_DIR, name)
        with open(path, "wb") as handle:
            handle.write(blob)
        total += 1
        print(f"{name}: {len(blob)} bytes")
    print(f"wrote {total} seeds to {CORPUS_DIR}")


if __name__ == "__main__":
    main()
