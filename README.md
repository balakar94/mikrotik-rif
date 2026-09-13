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
- **Follows your system.** Light/dark theme and interface language are detected
  from the OS and can be overridden per run.
- **Updates on your terms.** At most once a day the app asks the GitHub releases
  API whether a newer version exists (opt out in the footer). A dismissable
  banner offers the download, the installer is SHA-256-verified against the
  release's `SHA256SUMS.txt`, and the final install step is always yours.

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
| `Esc` | Close the search bar |

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

The interface language follows the system locale and falls back to English.
Translations are plain [Fluent](https://projectfluent.org/) files under
`locales/<tag>/mikrotik-rif.ftl`: the build script discovers them and embeds
them at compile time, with no registry to update by hand, and a test asserts
that every locale defines exactly the same identifiers as the base file. Set
`MIKROTIK_RIF_LANG` to force a locale for one run:

```sh
MIKROTIK_RIF_LANG=de cargo run -- supout.rif
```

## Layout

```
Cargo.toml
app.rc               Windows icon resource compiled into the .exe
LICENSE
.github/workflows/   CI, the tag-triggered release pipeline and a packaging smoke run
docs/RELEASE.md      release engineering and verified prerequisites
src/
  main.rs            entry point and module wiring
  app.rs             application shell, stages and orchestration
  app/workspace.rs   module rail, text view and in-module search
  app/update_ui.rs   update banner and its state machine
  splash.rs          welcome, home and the opening animation
  theme.rs           light/dark palettes, background gradient, fonts
  icons.rs           vector icons drawn with the painter
  i18n.rs            Fluent loading, OS detection, fallback chain
  panic.rs           last-resort panic log and dialog
  update.rs          release check, verified download and hand-off
  worker.rs          background reading, indexing and decoding
  parser/            capture format reader (codec, scanner, deflate, capture, limits, error)
locales/<tag>/mikrotik-rif.ftl
assets/fonts/        bundled Inter, JetBrains Mono and Noto Sans SC (OFL)
assets/icon/         application icon (PNG / ICNS / ICO)
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

## License

Released under the [MIT License](LICENSE). You are free to use, modify and
redistribute it, provided the copyright notice and permission notice are kept.

## Trademarks

This is an independent tool and is **not affiliated with, endorsed by or
sponsored by MikroTik**. "MikroTik" and "RouterOS" are trademarks of their
respective owner and are used here only to describe interoperability. The
router glyph in the application icon and on the welcome screen is a generic
access-point drawing, not the MikroTik logo.
