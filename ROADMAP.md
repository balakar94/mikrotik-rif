# Roadmap

Known, non-committal list of work that is intentionally not done yet. Status
words are honest about this repository's current state:

- **done** — shipped and verified.
- **implemented, off by default** — the code and tests exist, but the feature is
  inert until configured; the product does not use it yet.
- **implemented, exercised only by seed replay** — the harness and tests exist
  and run on the normal path, but the scheduled campaign is non-blocking and
  only seed replay has actually executed.
- **candidate** — worth considering, not scheduled.
- **not planned for now** — deliberately out of scope.

## Parser and tooling

- **Cooperative cancellation of indexing and expansion** — *done.* `Cancel`
  tokens are threaded through `from_bytes_cancellable`, `read_bytes` /
  `read_cached`, `expand_cancellable` and `locate_parts_cancellable`, and a
  raised token aborts with `RifError::Cancelled`. The worker trips the token of
  the in-flight index or expansion preemptively and reports `ExpandCancelled`.

- **Single-buffer indexing** — *done.* Each part body is transcoded once into
  one buffer and the compressed payload is recorded as an offset into it, so
  indexing no longer retains a second `to_vec` copy of every payload.

- **Bounded expansion cache** — *done.* `PartCache` is a least-recently-used
  cache bounded to 8 entries or 64 MiB of retained text by default, owned by the
  worker thread so a re-selected module is served without a second inflation.

- **Library surface, integration and property tests** — *done.* The parser is
  exposed as a library (`src/lib.rs`), with `tests/parser_integration.rs`,
  `tests/parser_properties.rs` and `tests/cancellation.rs`, plus an opt-in
  real-corpus test gated on `MIKROTIK_RIF_CORPUS`.

- **Fuzzing harness** — *implemented, exercised only by seed replay.* Four
  `cargo-fuzz` targets live under `fuzz/` with a committed seed corpus and a
  `verify_corpus` example; the scheduled workflow (`.github/workflows/fuzz.yml`)
  is non-blocking and only seed replay has actually executed.

- **Criterion benchmarks** — *done.* `benches/parser.rs` measures the scanner,
  indexing, the codec, expansion and the cache, and is declared with
  `harness = false` so the standard command measures for real.

- **Headless CLI** — *done.* The same binary answers `--help`/`-h`, `--list`
  and `--extract` (with `--module`/`--all` and `--out`/`--stdout`); a bare path
  or no arguments still opens the desktop viewer.

- **CLI capture size gate (512 MiB)** — *done.* `load_capture` in `src/cli.rs`
  rejects oversize inputs twice: once from file metadata before reading, once
  from the byte count after reading. Prevents OOM on huge captures.

- **ASCII fast-path in `filter_parts`** — *done.* `src/parser/capture.rs`
  skips case-fold allocation for pure-ASCII queries; non-ASCII (e.g. Ä/ä)
  still folds correctly. Covered by a unicode filter test.

- **Byte-granularity cancellation checkpoint** — *done.*
  `CANCEL_CHECK_BYTES` (1 MiB) in `src/parser/scanner.rs` trips the `Cancel`
  token on byte budget in addition to the existing per-line check, so long
  lines abort promptly.

## Release signing

- **Binary/release signing with minisign** — *implemented, off by default, not
  operational yet.*

  The updater already knows how to verify `SHA256SUMS.txt.minisig` when a public
  key is embedded at build time (`src/update.rs`, `verify_minisign`), and the
  release workflow signs the checksums with `minisign -S -W` when the
  `MINISIGN_SECRET_KEY` secret is present (`release.yml`,
  `Sign SHA256SUMS.txt (optional)`). **No key pair is configured**, so today
  releases ship unsigned and the updater verifies SHA-256 only; the signing path
  has never run on a real release. Enabling it requires:

  - a key pair generated with `minisign -G -W` (unencrypted, for CI),
  - the secret key stored as the `MINISIGN_SECRET_KEY` repository **secret**,
  - the public key stored as the `MIKROTIK_RIF_MINISIGN_PUBKEY` repository
    **variable**.

  Details and the exact commands live in [`docs/RELEASE.md`](docs/RELEASE.md).
  Note this is *release integrity*, not *code signing*: it does **not** remove
  the macOS Gatekeeper or Windows SmartScreen warnings.

- **OS code signing and notarization** (Apple Developer ID and notarization,
  Windows Authenticode) — *not planned for now.*

  Only this removes the unsigned-app warnings on first launch. It needs paid
  Apple and Microsoft certificates and, on macOS, a notarization step; the
  project currently ships unsigned on purpose and documents the manual
  open-anyway gesture in the README.

- **GitHub build-provenance attestations** (`actions/attest-build-provenance`,
  Sigstore via OIDC, no key custody) — *candidate.*

  Gives users verifiable build provenance with `gh attestation verify` and no
  private key to manage. It is not usable by the in-app updater, which would
  need a Sigstore verifier; it would complement, not replace, SHA-256 checks.

## Product and quality

- **Native review of the six non-English translations** (de, es, fr, lv, ru, zh)
  — *pending.* A test guarantees every locale defines every identifier; wording
  quality has not had a native-speaker pass. (0.4.0 added the missing
  `error-open-*`, `empty-no-readable` and `hint-find-keys` identifiers with
  best-effort translations.)

- **Workflow audit gate** (zizmor, high severity, annotations) and **secret
  scan** (gitleaks) — *done since 0.4.0.* Lower severities are surfaced as
  annotations for triage but do not block CI; the tree carries known pedantic
  hygiene findings (matrix expansions in build scripts, artifact credential
  persistence, dependabot cooldown).

- **Updater UI state-machine tests** — *candidate.* The network and verification
  logic is covered through a fake transport (`src/update/tests.rs`), but the
  `poll_update` transitions that decide what the Settings screen shows are not
  unit-tested.

- **AppImage host integration** for double-click `.rif` — *host-dependent.* The
  portable image carries the desktop entry and MIME XML but does not install
  them; a host integrator (e.g. `appimage-launcher`) is required.

- **Third-party notices** — *done.* `THIRD-PARTY-NOTICES.md` is generated with
  `cargo-about`, shipped inside every package, and checked in CI against
  `Cargo.lock`. The file embeds the crate version, so it must be regenerated
  in the same commit as every version bump.

- **Stage-fade repaint driver** — *done.* `FADE_SECONDS` in `src/app.rs`
  gates repaint requests while the stage transition fades, so a finished fade
  no longer freezes the frame.

- **Accessible roles on icon buttons and file chip** — *done.* `src/icons.rs`
  and the workspace `file_chip` expose `WidgetInfo` button roles for
  assistive tooling.

- **Updates tab without per-frame clones** — *done.*
  `src/app/settings/updates.rs` renders without cloning release state every
  frame.

- **Panic-log symlink guard** — *done.* `src/panic.rs` renames a pre-existing
  symlink aside instead of writing through it. Covered by
  `log_never_writes_through_a_symlink`.

- **Unified ETag trim** — *done.* `sanitize_etag` in `src/app.rs` trims
  surrounding whitespace before validation, matching the updater path.

- **Cache and path edge tests** — *done.* Byte-budget MRU eviction
  (`cache_evicts_by_bytes_preserving_mru`), cross-capture index isolation
  (`cached_index_no_reutiliza_entre_capturas`), stemless-path rejection
  (`default_parts_directory_rejects_stemless_path`), sanitised-stem
  collisions (`unique_stem_handles_sanitised_collisions`), endonym coverage
  for every shipped locale (`endonym_covers_every_shipped_locale`), plus a
  `TempDir` sequence test.

- **Crop-proof DMG background** — *done (0.8.2, supersedes the 0.8.1 pills).*
  Plain light gradient with distributed arcs and zero positioned art, after
  field evidence that resized Finder windows stretch the picture while icon
  slots stay fixed.

- **CLI/stdout and lookup perf** — *done (0.8.2).* Zero-copy
  `extract_to_stdout`, `Capture::find_first_named`, memoized build hash.

- **Workspace keyboard and cache** — *done (0.8.2).* Slash guard while
  typing, search populates the view cache, dynamic gutter width.

- **Locale and GUI-probe tests** — *done (0.8.2).*
  `fluent_resources_parse_without_errors`, `shipped_matches_filesystem`,
  liveness-based `bare_path` probe with headless skip.

- **CI drift guards and corpus gate** — *done (0.8.2).* Packaging maps,
  license allow-lists, `verify_corpus` plus regen check; `build.rs` watches
  `.git/refs/heads`.
