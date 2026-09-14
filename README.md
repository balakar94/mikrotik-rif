<h1 align="center">
  <img src="assets/icon/icon.png" alt="MikroTik RIF Viewer" width="128" height="128"><br>
  MikroTik RIF Viewer
</h1>

<p align="center">
  A desktop viewer for MikroTik RouterOS support captures (<code>supout.rif</code>).
</p>

<p align="center">
  <a href="https://github.com/balakar94/mikrotik-rif/actions/workflows/ci.yml"><img src="https://github.com/balakar94/mikrotik-rif/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT"></a>
</p>

A RouterOS support capture is a single text file that packs dozens of
diagnostic parts — the running configuration, interface and routing tables,
logs, and the output of many `/export` and diagnostic commands. It is what a
technician generates when something is wrong, and reading it normally means
scrolling through one enormous blob of text.

This viewer opens the file, indexes every part without unpacking all of it, and
presents them as a searchable module list beside the decoded text. It runs
entirely in the process: there is no backend, no upload step, and no network
traffic other than the opt-out update check described below.

- **Product name:** MikroTik RIF Viewer
- **Package / binary:** `mikrotik-rif`
- **Stack:** Rust (edition 2024) · [eframe/egui](https://github.com/emilk/egui) 0.36
- **License:** [MIT](LICENSE)

## Highlights

- **Indexes first, decodes on demand.** Opening a capture transcodes just enough
  of each part to learn its label and keeps the payload compressed. A part is
  inflated only when you select it, so peak memory tracks the open module rather
  than the whole file.
- **Responsive on huge captures.** Reading, indexing and decompression run on a
  background worker thread, and the module list and text view are virtualized,
  so a module with hundreds of thousands of lines still scrolls smoothly.
- **Honest about damage.** A part that cannot be indexed is still listed and
  marked, instead of being silently dropped.
- **Everything stays on the machine.** The parser is UI-agnostic and touches no
  files; the app writes nothing to disk unless you ask it to export a module.
- **Follows your system, remembers your choice.** Light/dark theme and
  interface language are detected from the OS and can be changed at any time in
  the Settings screen, where the choice is remembered across runs.
- **Updates on your terms.** On every launch, while it is enabled, the app asks
  the GitHub releases API whether a newer version exists (opt out in Settings).
  Settings shows the running version and a short build hash, offers the
  download, verifies the installer against the release's `SHA256SUMS.txt`, and
  always leaves the final install step to you — macOS only downloads the `.dmg`.
  **Skip this version** keeps the automatic check quiet until you run a manual
  check from Settings.

## Install

Pre-built packages are attached to every tagged release. They all share one
name, `mikrotik-rif_<version>_<arch>.<ext>`, where `<arch>` is `amd64` (64-bit
Intel/AMD) or `arm64`:

| OS | Architectures | Package | First launch |
| --- | --- | --- | --- |
| **Windows** | `amd64`, `arm64` | `-setup.exe` installer (NSIS) | Unsigned: SmartScreen may warn — **More info → Run anyway**. |
| **macOS** | Apple Silicon (`arm64`) only | `.dmg` (drag the app to Applications) | Unsigned: Gatekeeper blocks it — right-click the app → **Open**, or run `xattr -dr com.apple.quarantine "/Applications/MikroTik RIF Viewer.app"`. |
| **Debian / Ubuntu** | `amd64`, `arm64` | `.deb` | Installs a menu entry, an icon and the `.rif` file-type association. |
| **Fedora / RHEL** | `amd64`, `arm64` | `.rpm` | Same as `.deb`. |
| **Any Linux** | `amd64`, `arm64` | portable `.AppImage` | `chmod +x` and run it; no installation. |

The Windows, macOS and Linux packages register the `.rif` extension, so
double-clicking a capture opens it straight in the viewer (the path arrives as
`argv[1]`); the portable `.AppImage` relies on the host to integrate its
bundled MIME definition.

```sh
# Debian / Ubuntu
sudo apt install ./mikrotik-rif_<version>_amd64.deb

# Fedora / RHEL
sudo dnf install ./mikrotik-rif_<version>_amd64.rpm

# AppImage (any Linux distribution)
chmod +x mikrotik-rif_<version>_amd64.AppImage && ./mikrotik-rif_<version>_amd64.AppImage
```

Swap `amd64` for `arm64` on 64-bit ARM machines. Intel Macs are not shipped:
build from source there. Every download is listed in `SHA256SUMS.txt`, so you
can check it with `sha256sum -c SHA256SUMS.txt`.

These builds are intentionally unsigned. Code signing and notarization need paid
Apple and Windows certificates; they can be added later through CI secrets
without changing the pipeline.

## Using the app

The interface moves through four stages:

1. **Welcome.** The product name, a small diagram of the workflow (router →
   capture → technician) and a single **Start** action.
2. **Home.** Drag a `supout.rif` anywhere onto the window, or click the drop
   target to pick one.
3. **Opening.** While the capture is read and indexed, a page stack fans open
   under a magnifying glass. The animation tracks the real byte-read progress,
   then fades into the workspace.
4. **Workspace.** A **Modules** rail on the left with a name filter, and the
   decoded output of the selected module on the right.

In the workspace:

- The open capture is a chip in the top bar: click it (or `Cmd/Ctrl+O`) to open
  another file, so the action sits next to the file name it acts on.
- The **gear** on the right opens **Settings** (see below).
- The rail can be collapsed from the panel button in the top bar, giving the
  text the full width.
- Modules that share a label are disambiguated as `· copy 2`, `· copy 3`, and
  unreadable modules are listed in the alert colour.
- **Line numbers** toggles the gutter.
- The **magnifier** button opens in-module search; matches are highlighted and
  counted, with previous/next navigation. Enter jumps to the next match,
  Shift+Enter to the previous one, and Esc closes the bar.
- **Copy** puts the open module on the clipboard; **Save as…** writes it to a
  text file.

Keyboard shortcuts (Command on macOS, Control elsewhere):

| Shortcut | Action |
| --- | --- |
| `Cmd/Ctrl+O` | Open a capture |
| `Cmd/Ctrl+S` | Save the open module as… |
| `Cmd/Ctrl+F` | Search in the open module |
| `Cmd/Ctrl+,` | Open Settings |
| `Esc` | Close the search bar or Settings |

## Settings

The gear in the top bar (and on the welcome and home screens), or
`Cmd/Ctrl+,`, opens a modal with three tabs:

- **General** — theme (follow the system, light or dark) and interface language,
  both remembered across runs.
- **Updates** — the running version and a short build hash (SHA-256 of the
  compilation commit; the commit itself is in the tooltip), the automatic-check
  toggle and a manual **Check for updates**. The automatic check runs on every
  launch while enabled and reuses the previous response's ETag, so an unchanged
  release answers `304` instead of consuming the GitHub API quota; skipping a
  version keeps it quiet, and a manual check always surfaces it again. The tab
  also shows when the last check ran. When a newer release exists it shows its
  notes and one action: **Download and install** on Windows and Linux,
  **Download the .dmg** on macOS. A download is SHA-256-verified against the
  release's `SHA256SUMS.txt` before anything is handed to the operating system
  (and against its optional minisign signature when the build embeds a signing
  key), and a dot on the gear marks an update found by the automatic check.
- **About** — version, copyright, the MIT licence, a link to the GitHub
  repository and third-party credits (egui/eframe and the bundled fonts).

## Requirements

- Rust 1.85 or newer (edition 2024) to build.
- A Vulkan/Metal/DX12-capable GPU for the default eframe `wgpu` renderer.
- No runtime dependencies beyond the bundled fonts. On Linux, a desktop with
  X11 or Wayland.

## Build from source

```sh
cargo run                  # launch the app
cargo run -- supout.rif    # open a capture straight away
cargo build --release      # optimized binary in target/release/mikrotik-rif
```

Quality gates, the same ones CI runs:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The icon assets can be regenerated with `python3 tools/make_icon.py` (needs
Pillow; uses `iconutil` on macOS).

## Capture format

A capture is a text container. Each part is delimited by
`--BEGIN ROUTEROS SUPOUT SECTION` and `--END ROUTEROS SUPOUT SECTION` lines. The
body between them uses a base64-family alphabet (`A–Z a–z 0–9 + / =`) with an
unusual packing: each group of four symbols is read as a little-endian base-64
number and emitted least-significant byte first. The decoded bytes are a
NUL-terminated part label followed by a zlib stream.

The reader indexes labels only and inflates a single part on demand, under
configurable budgets:

| Limit | Default |
| --- | --- |
| Parts per capture | 100 000 |
| Raw part body | 128 MiB |
| Decompressed part | 256 MiB |
| Container line length | 64 MiB |

No real capture is committed to this repository: captures can carry sensitive
router configuration, so the tests build synthetic captures in memory.

## Localization

The interface language follows the system locale and falls back to English, and
can be switched at runtime from Settings. Translations are plain
[Fluent](https://projectfluent.org/) files under `locales/<tag>/mikrotik-rif.ftl`:
the build script discovers them and embeds them at compile time, with no
registry to update by hand, and a test asserts that every locale defines exactly
the same identifiers as the base file. Set `MIKROTIK_RIF_LANG` to force a locale
for one run:

```sh
MIKROTIK_RIF_LANG=de cargo run -- supout.rif
```

## Layout

```
Cargo.toml
app.rc               Windows icon resource compiled into the .exe
LICENSE
THIRD-PARTY-NOTICES.md  dependency licence texts, shipped inside every package
about.toml           cargo-about policy for the third-party notices
about.hbs            template used to render THIRD-PARTY-NOTICES.md
.github/workflows/   CI, the tag-triggered release pipeline and a packaging smoke run
docs/RELEASE.md      release engineering and verified prerequisites
src/
  main.rs            entry point and module wiring
  build_info.rs      version, git commit and the short build hash
  app.rs             application shell, stages and orchestration
  app/workspace.rs   module rail, text view and in-module search
  app/update_ui.rs   updater state machines and hand-off
  app/settings.rs    settings modal: shell, theme and language
  app/settings/      updates.rs and about.rs tabs
  splash.rs          welcome, home and the opening animation
  theme.rs           light/dark palettes, background gradient, fonts
  icons.rs           vector icons drawn with the painter
  i18n.rs            Fluent loading, OS detection, fallback chain
  panic.rs           last-resort panic log and dialog
  update.rs          updater's public surface, version and asset rules
  update/net.rs      HTTP transport, release fetch and installer download
  update/handoff.rs  installer hand-off and AppImage self-replace
  update/tests.rs    updater tests (fake transport, no network)
  worker.rs          background reading, indexing and decoding
  parser/            capture format reader (codec, scanner, deflate, capture, limits, error)
locales/<tag>/mikrotik-rif.ftl
assets/fonts/        bundled Inter, JetBrains Mono and Noto Sans SC (OFL)
assets/fonts/egui/   licence texts for egui's embedded default fonts
assets/icon/         application icon (PNG / ICNS / ICO / GitHub mark)
assets/linux/        freedesktop entry, MIME definition and 512 px icon
assets/macos/        disk image background
tools/make_icon.py   regenerates the icon assets
dist/                generated installers (git-ignored)
```

## Fonts

The interface bundles three fonts, all under the
[SIL Open Font License](https://openfontlicense.org/):

- [Inter](https://rsms.me/inter/) for the interface,
- [JetBrains Mono](https://www.jetbrains.com/lp/mono/) for capture text,
- [Noto Sans SC](https://fonts.google.com/noto/specimen/Noto+Sans+SC) as the CJK
  fallback, so Chinese (and any CJK content inside a capture) renders without
  relying on system fonts.

egui does not use system fonts, so every glyph the interface can show has to be
bundled. Inter covers Latin and Cyrillic; anything else falls through to Noto.
Each bundled font keeps its OFL text next to it in the source tree
(`assets/fonts/OFL-Inter.txt`, `OFL-JetBrainsMono.txt`, `OFL-NotoSansSC.txt`),
and the Linux packages install all three under
`/usr/share/licenses/mikrotik-rif/`.

Because the fonts are embedded, the release binary is around 19 MB, most of it
the CJK font.

## Roadmap

What is intentionally not done yet — including the minisign release-signing path
(code implemented, off by default, not operational until a key pair is
configured) and OS code signing — lives in [`ROADMAP.md`](ROADMAP.md).

## License

Released under the [MIT License](LICENSE). You are free to use, modify and
redistribute it, provided the copyright notice and permission notice are kept.

The binary also embeds third-party code and fonts. Their copyright notices and
licence texts are collected in
[`THIRD-PARTY-NOTICES.md`](THIRD-PARTY-NOTICES.md) and installed with every
package (regenerate with `cargo about generate about.hbs --output-file
THIRD-PARTY-NOTICES.md`; CI fails if the file drifts from `Cargo.lock`).

## Trademarks

This is an independent tool and is **not affiliated with, endorsed by or
sponsored by MikroTik**. "MikroTik" and "RouterOS" are trademarks of their
respective owner and are used here only to describe interoperability. The
router glyph in the application icon and on the welcome screen is a generic
access-point drawing, not the MikroTik logo.

The GitHub mark in the About tab is GitHub's official logo, unmodified and used
only as a link to this project's repository; the project is not affiliated with
or endorsed by GitHub.
