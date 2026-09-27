# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
