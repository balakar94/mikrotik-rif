# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versioning
follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
