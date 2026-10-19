# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- Updater: `rustls` 0.23.45 → 0.23.46, closing four RustSec advisories on the
  TLS stack behind the opt-in update check — RUSTSEC-2026-0346 (`Debug` on
  client session values exposed TLS secrets), RUSTSEC-2026-0347 (buffered
  application data sent after ECH rejection), RUSTSEC-2026-0348 (Brotli
  certificate decompression could allocate 1 GiB before authentication) and
  RUSTSEC-2026-0349 (unauthenticated ServerHello could downgrade later
  connections to classical key exchange).
- `deny.toml`: record RUSTSEC-2026-0370/0371/0372 (`zvariant`, `zbus_names`
  and `zvariant_derive` 5.x unmaintained) with explicit follow-up reasons.
  All three carry `patched = []` — no release can satisfy them — and the only
  way out is a future `accesskit_unix`/egui release that moves off zbus 5
  (0.24 still requires `zbus ^5.19`). They are Linux-only (AT-SPI
  accessibility) but stay visible as ignored rather than hidden.
- Panic log: the file is now created exclusively (`O_CREAT|O_EXCL`, mode
  `0600`) and an existing path is only opened after confirming it is not a
  symlink; the opened handle is compared against the inspected inode, so a
  link planted in the shared temporary directory can no longer redirect
  panic text (or a chmod/truncate) into another file.
- Viewer: replace the unbounded line-start vector with a checkpointed
  `LineIndex` that keeps one offset per 1024 lines and scans forward on
  demand. A part made only of newlines expands to as much as 256 MiB but
  compresses to a few hundred KiB; the old index allocated 8 bytes per line
  (up to ~2 GiB) on the UI thread, an infallible allocation failure that
  aborts the process before the panic hook can run. The index is built lazily
  (the global search no longer indexes every module it scans) and its
  footprint is counted in the view-cache byte budget.
- CLI: `--extract` now enforces a 1 GiB aggregate cap on expanded output
  (`MAX_EXTRACT_BYTES`), so `--all` over a 512 MiB capture of highly
  compressible modules can no longer write hundreds of GiB to
  `--out`/`--stdout`. The cap fails closed: the first part that would cross
  it is refused with an operational error (exit 1) before its output file is
  created, while output already written stays in place.
- Release: generate the CycloneDX SBOM in a dedicated `contents: read` job
  and hand it to the publishing job as an artifact. `cargo install` compiles
  and runs a third-party build script, so it no longer executes in the job
  that holds the write token, the OIDC grant and `MINISIGN_SECRET_KEY`.

### Fixed

- CI: pin `dtolnay/rust-toolchain` to a `master` commit instead of the
  `stable` branch. dtolnay force-rewrites `stable` on every Rust release, so
  the pinned commit stops being reachable from any ref and zizmor's
  `impostor-commit` audit failed every run — the second occurrence after
  3e5fd9e. `toolchain:` is now passed explicitly at each call site, since
  `master`'s `action.yml` marks that input required.
- CI: correct the MSRV job comment, which claimed the `test` matrix covers
  newer toolchains; `rust-toolchain.toml` pins channel 1.95 for local
  development, and the matrix now overrides it with `RUSTUP_TOOLCHAIN:
  stable`.
- CI: correct the fuzz workflow comment claiming the toolchain action exports
  `RUSTUP_TOOLCHAIN`; it only runs `rustup default`, which
  `rust-toolchain.toml` overrides, so the steps use `cargo +nightly`.
- Worker: `Event::Expanded` carries the cache's `Arc<PartText>` instead of
  `Arc::unwrap_or_clone`, so a cache hit no longer deep-copies a payload that
  can reach `CaptureLimits::max_part_bytes` (256 MiB by default).
- File names: Windows-reserved stems are matched after stripping trailing dots
  and spaces, so `CON .txt` and `COM1. .log` no longer pass as ordinary names.
- CLI: `--list`/`--extract` refuse a path that is not a regular file
  (directory, FIFO, device) instead of reading it; `fs::read` on `/dev/zero`
  had no cap and could grow without bound.
- Updater: `check_url` refuses an explicit non-443 port and matches the host
  ASCII case-insensitively, so `https://github.com:8443/…` is no longer
  accepted on the strength of its host name.

### Changed

- Updater: the check/download state and the `poll` transitions moved from
  `Viewer` into a standalone `Updater` struct, so they can be exercised
  without an `eframe` context or a network. Fifteen unit tests now cover an
  offered release, a skipped tag (quiet on automatic checks, shown on manual
  ones), `304 Not Modified`, up-to-date answers, manual vs automatic failures,
  dropped channels, stale installers, and newest-progress-wins download
  draining.
- CI: the `test` matrix builds and tests with the newest stable Rust on all
  three OSes (`RUSTUP_TOOLCHAIN: stable` at job level), while the `msrv` job
  remains the 1.95 gate and now also runs `cargo +1.95 fmt --all --check`, so
  formatting stays tied to the toolchain pinned by `rust-toolchain.toml`.
- Dependencies: `zbus_xml` 5.1.0 → 5.2.1 (with `zbus_names` and `zvariant`),
  which replaces `quick-xml` with `winnow` and drops `quick-xml` 0.38.4 from
  the lock, clearing RUSTSEC-2026-0194 and RUSTSEC-2026-0195 outright instead
  of ignoring them.

## [0.9.0] - 2026-10-05

### Fixed

- CI: fuzz corpus gate runs without `--locked` (no committed fuzz lockfile).
- Tests: satisfy `map_unwrap_or` lint in the GUI probe.
- Worker: expansion cache is owned per capture, so an `Expand` queued before
  a new `Index` can no longer serve stale text under the new capture.
- Linux MIME: `application/x-mikrotik-rif` also matches `*.RIF` uppercase
  (Linux globs are case-sensitive).

## [0.8.2] - 2026-09-27

### Added

- `Capture::find_first_named` for exact first-match module lookup.
- CI guards: packaging file-map sync, license allow-list sync, fuzz corpus
  replay plus regen check on Linux.
- Tests: Fluent resources parse without errors, `SHIPPED` matches the
  `locales/` filesystem, GUI liveness probe with headless skip.

### Changed

- DMG background is a plain light gradient with distributed arcs and no
  positioned art: v0.8.1 white pills drifted off the labels in resized
  Finder windows, so nothing is load-bearing at the pixel level anymore.

### Fixed

- `--extract --stdout` streams header then body instead of duplicating the
  part text in memory.
- `/` shortcut no longer steals focus while typing in a text field.
- Global-search expansions populate the view cache, so opening a hit is
  instant; the scan skips already-cached modules.
- Gutter width derives from the line count instead of a fixed 8 columns.
- Build hash memoized instead of re-hashing per frame in the Updates tab.
- Local builds watch `.git/refs/heads` besides `.git/HEAD` for the embedded
  commit.

## [0.8.1] - 2026-09-27

### Fixed

- DMG Finder labels legible: white pills behind the app and Applications
  name zones, arrow repositioned between the icons (`assets/macos/`).
- Dependency bumps: `thiserror` 2.0.20->2.0.21, `ureq` 3.4.1->3.4.2
  (`ureq-proto` 0.6.2->0.6.4); notices regenerated.
- Release workflow: `actions/attest-build-provenance` 3.0.0->4.2.2
  (drop-in, `subject-path` unchanged).

## [0.8.0] - 2026-09-27

### Added

- Byte-granularity cancellation checkpoint (`CANCEL_CHECK_BYTES`, 1 MiB) in
  `src/parser/scanner.rs`, in addition to the per-line check.
- ASCII fast-path in `filter_parts` (`src/parser/capture.rs`) with unicode
  (Ä/ä) coverage.
- Accessible button roles (`WidgetInfo`) on icon buttons (`src/icons.rs`) and
  the workspace `file_chip`.
- Single owner of file-name sanitizing in `src/filenames.rs` (`sanitize`,
  `escape_label`); `cli` and the workspace reuse it.
- Retina DMG background: `assets/macos/dmg-background.svg` master plus
  1320x800 `@2x` PNG render, rings on the icon slots.
- Tests: byte-budget MRU eviction, cross-capture index isolation, CLI
  stemless-path and sanitised-collision edges, endonym coverage for every
  shipped locale, `TempDir` sequence, panic symlink refusal, stdout
  header-escape with a hostile label.

### Fixed

- CLI capture size gate: `load_capture` in `src/cli.rs` rejects files over
  512 MiB from metadata and from the post-read byte count.
- Finished stage fade no longer freezes the frame; repaint is gated on
  `FADE_SECONDS` in `src/app.rs`.
- Updates tab (`src/app/settings/updates.rs`) renders without cloning release
  state every frame.
- Panic log (`src/panic.rs`) renames a pre-existing symlink aside instead of
  writing through it.
- ETag sanitization trims surrounding whitespace in `src/app.rs`, matching
  the updater path.
- `extract_to_directory` refuses symlinks and uses exclusive creation with a
  numeric-suffix fallback instead of silently truncating; `--list` and
  `--extract --stdout` headers escape control/ANSI bytes in labels.

## [0.7.0] - 2026-09-19

### Added

- The capture reader is also a library now (`src/lib.rs` exposes `pub mod
  parser`); `src/main.rs` is a thin wrapper around it. Integration tests, fuzz
  targets, benchmarks and the headless CLI all build against the same surface.
- Cooperative cancellation for indexing and expansion: `Cancel` tokens,
  `Capture::from_bytes_cancellable`, `read_bytes`/`read_cached`,
  `deflate::expand_cancellable` and `scanner::locate_parts_cancellable`, with a
  new `RifError::Cancelled` that aborts the whole operation instead of being
  downgraded to a per-part fault.
- Raw decompressed bytes are reachable through `Capture::read_bytes`, so export
  and byte-oriented callers can skip UTF-8 decoding.
- Worker: preemptive cancellation of the in-flight index pass and expansion, a
  worker-owned `PartCache`, the additive `Event::ExpandProgress` and
  `Event::ExpandCancelled`, plus `Worker::cancel_expand` and
  `Worker::cache_stats`.
- A bounded least-recently-used expansion cache (`PartCache`, 8 entries or
  64 MiB of retained text by default), so re-selecting a module is served
  without a second inflation.
- Structural container notes (`Capture::notes`) are surfaced in a dismissible
  banner with a bounded detail list, and a search across every readable module
  of the open capture runs on the worker with progress and cancellation.
- Expanded modules are kept in a bounded view cache, so revisiting a module
  opened earlier in the session is a shared-reference clone rather than a second
  expansion.
- Headless CLI in the same binary: `--help`/`-h`, `--list` and `--extract`
  (`--module`/`--all`, `--out`/`--stdout`). A bare path still opens the viewer,
  and exit codes distinguish success, operational failure and a malformed
  command line.
- Parser integration, property and cancellation tests under `tests/`, an opt-in
  real-corpus test gated on `MIKROTIK_RIF_CORPUS`, a four-target `cargo-fuzz`
  harness under `fuzz/`, Criterion benchmarks under `benches/`, and a
  non-blocking scheduled fuzz workflow.
- Fluent identifiers for the notes banner and the global search were added to
  all seven locales.

### Changed

- Indexing transcodes each part body once into a single buffer and records the
  compressed payload as an offset into it, instead of retaining a second copy of
  every payload.
- `max_line_bytes` is raised to 128 MiB so it matches `max_span_bytes`: a
  single-line part body that fits the span budget can no longer be rejected by
  the narrower line budget.
- The pure view kernels `compute_view`, `next_match` and `filter_parts` are
  re-exported from `parser`, so they can be exercised without a UI toolkit.

### Fixed

- Removed the dead `PartUnreadable` error variant.

## [0.4.2] - 2026-09-16

### Fixed

- Gutter toggle back to the sidebar icon only; the wide-layout "Line numbers"
  text button is gone.
- Removed the raw release-notes excerpt from the Updates tab, which rendered
  unformatted markdown.

### Added

- Standalone "What's new" window (Settings → Updates) rendering the bundled
  changelog: resizable, independent of the settings modal, no network needed.

## [0.4.1] - 2026-09-16

### Fixed

- Release plumbing only, no product changes: the SBOM step invoked
  cargo-cyclonedx 0.5.5 with a nonexistent `--output` flag, which failed the
  v0.4.0 publish job after all installers had built. The step now uses
  `--override-filename` plus a move into `dist/`. Per the immutable-tag
  policy, v0.4.0 (no published release) is superseded by this patch.

## [0.4.0] - 2026-09-16

### Added

- Background worker with per-worker cancellation: truly preemptive `cancel()`,
  stale expansions dropped via sequence numbers (`src/worker.rs`).
- Keyboard-operable home drop-zone with an explicit Open button, a visible
  gutter label, and F3/Cmd+G gated on an open find bar.
- Localized validation errors (`error-open-*` keys) in all seven locales, and
  translations for the previously English-only `empty-no-readable` and
  `hint-find-keys` strings.
- CI gates: zizmor workflow audit (high severity, annotations), gitleaks
  secret scan, MSRV 1.95 check, and `cargo package --list` hygiene.

### Fixed

- Updater verification: minisign over `SHA256SUMS.txt` with streaming SHA-256
  and reverify at handoff, 512 MiB installer / 1 MiB metadata caps, ETag
  sanitization, path-traversal stripping, and symlink-safe destinations.
- Parser budgets: capped `from_reader`, accumulated payload totals, early
  `TooManyParts` abort, and zip-bomb containment in zlib expansion.
- Pinned Rust 1.95 toolchain; SHA-pinned audit and attestation actions;
  release workflow defaults to `contents: read`.

### Security

- Bump rustls 0.23.44 to 0.23.45 fixing RUSTSEC-2026-0285 (TLS 1.3 handshake
  messages accepted across encryption level boundaries), via the ureq
  dependency.

### Release integrity

- Releases ship **unsigned**: no minisign key pair is configured, so the
  updater verifies SHA-256 over TLS only. See `ROADMAP.md` (Release signing)
  for what enabling signatures requires.
