#![forbid(unsafe_code)]
//! Criterion benchmarks for the capture parser's hot paths.
//!
//! Every fixture is synthesised in memory from the inverse of the reader:
//! `label`, a `NUL` separator, a zlib payload built with `flate2`, the whole
//! body transcoded with [`mikrotik_rif::parser::codec::pack`] and wrapped in the
//! container markers. No real capture is committed, because captures carry
//! sensitive router configuration.
//!
//! # What each benchmark guards
//!
//! * [`bench_scanner`] — the linear marker walk. Guards against an accidental
//!   per-line allocation or repeated scan of the source as the part count grows.
//! * [`bench_indexing`] — [`Capture::from_bytes`]. Guards the single-allocation
//!   transcoding design: the removed `to_vec` copy used to retain a second copy
//!   of every payload, so indexing throughput must scale with the transcoded
//!   bytes, not with a duplicate of them.
//! * [`bench_codec`] — the group-by-group envelope decoder. Guards against a
//!   second counting pass or an unbounded eager reservation/`shrink_to_fit`
//!   churn.
//! * [`bench_read`] — [`Capture::read`], [`Capture::read_bytes`] and
//!   [`Capture::read_cached`] on a cache miss and a cache hit. Guards the
//!   expansion budget check and, above all, the cache: a hit must cost an
//!   `Arc` clone rather than a fresh inflate.
//! * [`bench_deflate`] — the raw zlib expander on compressible and
//!   incompressible payloads. Guards the removal of an accidental whole-payload
//!   copy before inflating and keeps the incompressible path honest.
//!
//! The two payload shapes (`compressible` and `incompressible`) are a
//! [`BenchmarkId`] dimension wherever both affect the measured path, so a
//! regression that only hurts one shape cannot hide in an average.

use std::hint::black_box;
use std::io::Write;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use flate2::Compression;
use flate2::write::ZlibEncoder;

use mikrotik_rif::parser::codec;
use mikrotik_rif::parser::deflate;
use mikrotik_rif::parser::scanner::{CLOSE_MARKER, OPEN_MARKER, locate_parts};
use mikrotik_rif::parser::{Cancel, Capture, CaptureLimits, PartCache};

/// Body size of every part in the many-part captures.
const SMALL_BODY: usize = 256;

/// Decompressed size of the large single-part captures (2 MiB).
const LARGE_BODY: usize = 2 * 1024 * 1024;

/// Raw size of the body fed to the `codec` benchmarks (1 MiB).
const CODEC_BODY: usize = 1024 * 1024;

/// Fixed seeds keep the incompressible fixtures identical between runs.
const SEED_READ: u64 = 0x1234_5678_9ABC_DEF0;
const SEED_CODEC: u64 = 0x0FED_CBA9_8765_4321;

/// An indexed single-part capture and the limits it was built with.
///
/// The capture owns its transcoded buffers, so the source bytes are not kept
/// here: dropping them proves the read paths borrow nothing from the fixture.
struct Fixture {
    capture: Capture,
    limits: CaptureLimits,
    plain_len: usize,
}

impl Fixture {
    /// Build and index one part carrying `body`.
    fn single_part(label: &[u8], body: &[u8]) -> Self {
        let source = wrap(&[encode_part(label, body)]);
        let limits = CaptureLimits::default();
        let capture = Capture::from_bytes(&source, &limits).expect("synthetic fixture must index");
        Self {
            capture,
            limits,
            plain_len: body.len(),
        }
    }

    /// Compressed payload of the single part, exactly what the reader inflates.
    fn payload(&self) -> &[u8] {
        self.capture.parts()[0].payload()
    }
}

/// Compress `bytes` into the zlib stream a real part carries.
fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(bytes)
        .expect("writing to an in-memory encoder cannot fail");
    encoder
        .finish()
        .expect("finishing an in-memory encoder cannot fail")
}

/// Encode one part the way the router would: label, `NUL`, zlib payload.
fn encode_part(label: &[u8], body: &[u8]) -> Vec<u8> {
    let mut plain = Vec::with_capacity(label.len() + 1 + body.len());
    plain.extend_from_slice(label);
    plain.push(0);
    plain.extend_from_slice(&zlib(body));
    codec::pack(&plain)
}

/// Wrap already-encoded parts in the container markers.
fn wrap(parts: &[Vec<u8>]) -> Vec<u8> {
    let mut source = Vec::new();
    for part in parts {
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(part);
        source.push(b'\n');
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
    }
    source
}

/// Build a capture holding `parts` copies of one small, compressible part.
///
/// Reusing the encoded body keeps fixture construction O(parts) in bytes copied
/// rather than re-deflating the same body thousands of times.
fn many_part_source(parts: usize, body: &[u8]) -> Vec<u8> {
    let encoded = encode_part(b"part", body);
    let per_part = OPEN_MARKER.len() + encoded.len() + CLOSE_MARKER.len() + 3;
    let mut source = Vec::with_capacity(parts.saturating_mul(per_part));
    for _ in 0..parts {
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(&encoded);
        source.push(b'\n');
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
    }
    source
}

/// Deterministic, compressible RouterOS-like output of exactly `len` bytes.
fn compressible(len: usize) -> Vec<u8> {
    const LINE: &[u8] =
        b"/interface ethernet print detail\r\n  name=ether1 rx-byte=123456 tx-byte=654321\r\n";
    LINE.iter().copied().cycle().take(len).collect()
}

/// Deterministic incompressible bytes: a 64-bit LCG spilling little-endian
/// words, so zlib cannot find repetition to exploit.
fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// Guard the marker walk against per-part work growing beyond the number of
/// bytes scanned.
fn bench_scanner(c: &mut Criterion) {
    let limits = CaptureLimits::default();
    let body = compressible(SMALL_BODY);
    let mut group = c.benchmark_group("scanner::locate_parts");

    for parts in [1_000usize, 10_000] {
        let source = many_part_source(parts, &body);
        let count = u64::try_from(parts).expect("part count fits in u64");
        group.throughput(Throughput::Elements(count));
        group.bench_with_input(BenchmarkId::new("parts", parts), &source, |b, source| {
            b.iter(|| {
                let mut notes = Vec::new();
                let spans = locate_parts(source, &limits, &mut notes).expect("fixture must scan");
                black_box(spans)
            });
        });
    }

    group.finish();
}

/// Guard the indexing path against a second copy of every payload and against
/// span-count overhead.
fn bench_indexing(c: &mut Criterion) {
    let limits = CaptureLimits::default();
    let small = compressible(SMALL_BODY);
    let large = pseudo_random(LARGE_BODY, SEED_READ);

    let single = wrap(&[encode_part(b"export", &large)]);
    let one_k = many_part_source(1_000, &small);
    let ten_k = many_part_source(10_000, &small);
    let cases = [
        ("large-single-part", &single),
        ("many-parts-1k", &one_k),
        ("many-parts-10k", &ten_k),
    ];

    let mut group = c.benchmark_group("Capture::from_bytes");
    for (shape, source) in cases {
        group.throughput(Throughput::Bytes(
            u64::try_from(source.len()).expect("source length fits in u64"),
        ));
        group.bench_with_input(BenchmarkId::from_parameter(shape), source, |b, source| {
            b.iter(|| black_box(Capture::from_bytes(source, &limits).expect("fixture must index")));
        });
    }
    group.finish();
}

/// Guard the transcoder against a counting pre-pass or capacity thrash.
fn bench_codec(c: &mut Criterion) {
    let raw = pseudo_random(CODEC_BODY, SEED_CODEC);
    let encoded = codec::pack(&raw);
    let mut group = c.benchmark_group("codec");
    group.throughput(Throughput::Bytes(
        u64::try_from(encoded.len()).expect("encoded length fits in u64"),
    ));

    group.bench_with_input(
        BenchmarkId::from_parameter("unpack"),
        &encoded,
        |b, encoded| b.iter(|| black_box(codec::unpack(encoded).expect("fixture must unpack"))),
    );
    group.bench_with_input(
        BenchmarkId::from_parameter("unpack_capped"),
        &encoded,
        |b, encoded| {
            b.iter(|| {
                black_box(codec::unpack_capped(encoded, usize::MAX).expect("fixture must unpack"))
            });
        },
    );

    group.finish();
}

/// Guard expansion and, above all, the part cache: a hit must not inflate.
fn bench_read(c: &mut Criterion) {
    let cases = payload_cases();
    let cancel = Cancel::default();
    let mut group = c.benchmark_group("Capture::read");

    for (shape, fixture) in &cases {
        group.throughput(Throughput::Bytes(
            u64::try_from(fixture.plain_len).expect("body length fits in u64"),
        ));

        group.bench_with_input(BenchmarkId::new("text", shape), fixture, |b, f| {
            b.iter(|| black_box(f.capture.read(0, &f.limits).expect("fixture must read")));
        });
        group.bench_with_input(BenchmarkId::new("bytes", shape), fixture, |b, f| {
            b.iter(|| {
                black_box(
                    f.capture
                        .read_bytes(0, &f.limits, &cancel)
                        .expect("fixture must read"),
                )
            });
        });

        let mut miss_cache = PartCache::default();
        group.bench_with_input(BenchmarkId::new("cached_miss", shape), fixture, |b, f| {
            b.iter(|| {
                miss_cache.clear();
                black_box(
                    f.capture
                        .read_cached(0, &f.limits, &mut miss_cache, &cancel)
                        .expect("fixture must read"),
                )
            });
        });
    }

    for (shape, fixture) in &cases {
        let mut hit_cache = PartCache::default();
        fixture
            .capture
            .read_cached(0, &fixture.limits, &mut hit_cache, &cancel)
            .expect("priming the cache must succeed");
        group.throughput(Throughput::Bytes(
            u64::try_from(fixture.plain_len).expect("body length fits in u64"),
        ));
        group.bench_with_input(BenchmarkId::new("cached_hit", shape), fixture, |b, f| {
            b.iter(|| {
                black_box(
                    f.capture
                        .read_cached(0, &f.limits, &mut hit_cache, &cancel)
                        .expect("fixture must read"),
                )
            });
        });
    }

    group.finish();
}

/// Guard the raw expander on both payload shapes, so an incompressible input
/// cannot hide a copy or a repeated read.
fn bench_deflate(c: &mut Criterion) {
    let cases = payload_cases();
    let mut group = c.benchmark_group("deflate::expand");

    for (shape, fixture) in &cases {
        group.throughput(Throughput::Bytes(
            u64::try_from(fixture.plain_len).expect("body length fits in u64"),
        ));
        group.bench_with_input(BenchmarkId::from_parameter(shape), fixture, |b, f| {
            let payload = f.payload();
            b.iter(|| {
                black_box(
                    deflate::expand(payload, f.limits.max_part_bytes).expect("fixture must expand"),
                )
            });
        });
    }

    group.finish();
}

/// One large single-part fixture per payload shape, shared by the read and
/// expander groups.
fn payload_cases() -> [(&'static str, Fixture); 2] {
    let compressible_body = compressible(LARGE_BODY);
    let incompressible_body = pseudo_random(LARGE_BODY, SEED_READ);
    [
        (
            "compressible",
            Fixture::single_part(b"export", &compressible_body),
        ),
        (
            "incompressible",
            Fixture::single_part(b"export", &incompressible_body),
        ),
    ]
}

criterion_group!(
    benches,
    bench_scanner,
    bench_indexing,
    bench_codec,
    bench_read,
    bench_deflate
);
criterion_main!(benches);
