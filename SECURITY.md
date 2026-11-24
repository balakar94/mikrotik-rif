# Security policy

## Scope

The parts that handle untrusted or attacker-influenced input:

- **Capture parser** (`src/parser/`): reads a `.rif` file that may come from an
  untrusted router or a third party. It must not panic, over-read, allocate
  without bounds, or write files.
- **Updater** (`src/update.rs`, `src/update/`): parses GitHub release metadata,
  follows redirects and downloads installers. It must keep the host allow-list,
  verify SHA-256 against `SHA256SUMS.txt`, verify `SHA256SUMS.txt.minisig` when
  a public key is embedded, and never execute an unverified file.
- **Headless CLI** (`src/cli.rs`): expands modules to `--out` directories. It
  must keep label/path sanitisation and the byte and file-count caps.

Packaging and CI are in scope when they weaken any of the above. Out of scope:
bugs that need an already-compromised local account, and the absence of OS code
signing (documented in [`ROADMAP.md`](ROADMAP.md); releases are unsigned by
design).

## Supported versions

Security fixes are shipped for the latest `1.x` release. Older `1.x` releases
may be patched at the maintainer's discretion; pre-1.0 releases are not
supported. Because the in-app updater only offers newer stable releases,
running the latest `1.x` is the practical way to receive a fix.

| Version | Supported |
| --- | --- |
| Latest `1.x` | Yes |
| Older `1.x` | Best effort |
| `< 1.0` | No |

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: go to the repository's
**Security** tab and choose **Report a vulnerability** (GitHub Security
Advisories). That channel is private between you and the maintainer.

**Do not open a public issue for a security problem**, and do not attach a real
capture to a public thread.

Please include:

- the app version (`mikrotik-rif --version`, or Settings → About) and, if you
  built from source, the commit;
- OS and architecture, and how the app was installed (installer, package,
  AppImage, from source);
- a minimal reproducer: the command or UI path, and either a sanitised capture
  or the generator/script that produces the input;
- whether you can share a capture privately (captures often contain router
  configuration, so a synthetic one is preferred);
- the impact you expect, and any suggested fix.

The maintainer will acknowledge the report as soon as possible, keep you
informed while it is investigated, and coordinate disclosure in an advisory
with a fixed release. This is a volunteer project: there is no SLA and no
bug-bounty programme. Credit is given in the advisory unless you ask otherwise.

## Threat model notes

- **Captures are untrusted input.** The parser uses explicit limits for parts,
  line length, per-part bytes and total transcoded bytes, and is exercised by
  unit, property and fuzz targets. A capture that can crash or exhaust the
  process is a security bug worth reporting.
- **Update metadata and installers are untrusted transport.** The updater
  accepts only `https://` URLs on an allow-list (`github.com`, `api.github.com`,
  `objects.githubusercontent.com`, `release-assets.githubusercontent.com`),
  re-validates every redirect hop, and verifies the downloaded installer against
  the release's `SHA256SUMS.txt` before handing it to the OS. When release
  signing is configured, a valid minisign signature is also required. SHA-256
  checks and build-provenance attestations give integrity and build-origin
  evidence; they do not protect against a compromised maintainer account.
- **No telemetry.** The app has no analytics, accounts or background services;
  the opt-out update check is described in [`PRIVACY.md`](PRIVACY.md).
- **Builds are unsigned.** Gatekeeper and SmartScreen warnings are expected;
  that is a code-signing gap, not a vulnerability report.
