# Roadmap

Known, non-committal list of work that is intentionally not done yet. Status
words are honest about this repository's current state:

- **done** — shipped and verified.
- **implemented, off by default** — the code and tests exist, but the feature is
  inert until configured; the product does not use it yet.
- **candidate** — worth considering, not scheduled.
- **not planned for now** — deliberately out of scope.

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
  quality has not had a native-speaker pass.

- **Updater UI state-machine tests** — *candidate.* The network and verification
  logic is covered through a fake transport (`src/update/tests.rs`), but the
  `poll_update` transitions that decide what the Settings screen shows are not
  unit-tested.

- **AppImage host integration** for double-click `.rif` — *host-dependent.* The
  portable image carries the desktop entry and MIME XML but does not install
  them; a host integrator (e.g. `appimage-launcher`) is required.

- **Third-party notices** — *done.* `THIRD-PARTY-NOTICES.md` is generated with
  `cargo-about`, shipped inside every package, and checked in CI against
  `Cargo.lock`.
