# Parser benchmarks

Criterion benchmarks for the capture parser's hot paths. Every fixture is
synthetic and built in memory (label + `NUL` + zlib via `flate2` + `codec::pack`
wrapped in the container markers); no real capture is committed.

`Cargo.toml` declares the target as a Criterion harness:

```toml
[[bench]]
name = "parser"
harness = false
```

so `criterion_main!` owns the process and the standard command measures for
real.

## Running

```sh
cargo bench --bench parser
```

Short, CI-friendly run:

```sh
cargo bench --bench parser -- --warm-up-time 0.5 --measurement-time 1 --sample-size 10
```

## What to watch

- **Indexing throughput vs. part count.** `Capture::from_bytes` must scale with
  the transcoded bytes, not with a per-part copy. A regression in the removed
  `to_vec` path shows up as indexing time growing faster than the source length
  for `many-parts-10k` relative to `large-single-part`.
- **Expansion throughput, incompressible vs. compressible.** `deflate::expand`
  and `Capture::read*` should be dominated by inflating the output. If an
  incompressible payload regresses while the compressible one does not, suspect
  an extra copy of the payload rather than the zlib code itself.
- **Cache-hit cost.** `cached_hit` must stay near an `Arc` clone and far below
  `cached_miss`/`text`; if they converge, the cache is no longer short-circuiting
  expansion. `cached_miss` uses a cache that is cleared each iteration so it
  exercises the full expand-and-insert path.
- **Scanner part-count scaling.** `locate_parts` is a linear walk; its
  throughput in parts/s should be roughly flat between 1k and 10k parts.

## Caveats

Numbers are local and machine-dependent. The fixtures are small by design
(256 B parts; 2 MiB large bodies; 1 MiB codec body) so the whole suite finishes
in a couple of minutes on a developer laptop; they are not a load test and do
not represent a real multi-hundred-megabyte capture. Always compare against a
baseline measured on the same machine, toolchain and profile.
