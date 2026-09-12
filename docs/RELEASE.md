# Release engineering

This document describes how `mikrotik-rif` is built and released by GitHub
Actions. The workflows live in `.github/workflows/`.

## Workflows

| Workflow | File | Trigger | Purpose |
| --- | --- | --- | --- |
| CI | `.github/workflows/ci.yml` | `push` to `main`, `pull_request`, `workflow_dispatch` | Format, lint and test on Linux, macOS and Windows. |
| Release | `.github/workflows/release.yml` | `push` of tags matching `v*`, `workflow_dispatch` | Build native installers and, for tags, publish a GitHub Release. |

CI installs the Linux development packages eframe/winit/wgpu need. The release
workflow builds the application once per runner (`cargo build --release --locked`)
and lets cargo-packager consume that binary; it does **not** compile the app
again. Each architecture is built on its own **native** arm64/x64 runner (no
cross-compilation).

## Artifacts produced per runner and architecture

| Runner | Architecture | Tool | Formats (arch appears in the file name) |
| --- | --- | --- | --- |
| `windows-latest` | x64 | cargo-packager | WiX `.msi`, NSIS `-setup.exe` (`..._x64...`) |
| `windows-11-arm` | arm64 | cargo-packager | WiX `.msi`, NSIS `-setup.exe` (`..._arm64...`) |
| `macos-latest` | arm64 (Apple Silicon) | cargo-packager | `.app` bundle, `.dmg`; the `.app` is additionally zipped as `MikroTik-RIF-Viewer.app.zip` |
| `ubuntu-latest` | x64 | cargo-packager | `.deb` (`amd64`), `.AppImage` (`x86_64`) |
| `ubuntu-24.04-arm` | arm64 | cargo-packager | `.deb` (`arm64`), `.AppImage` (`aarch64`) |
| `ubuntu-latest` | x64 | cargo-generate-rpm | `.rpm` (`x86_64`) |
| `ubuntu-24.04-arm` | arm64 | cargo-generate-rpm | `.rpm` (`aarch64`) |

Because cargo-packager and cargo-generate-rpm include the target architecture
in every installer name, the x64 and arm64 artifacts never collide when the
release job merges them into a single `dist/` directory.

All builds are **unsigned**. No signing certificates or secrets are used.

## Linux desktop integration

Every Linux package installs a freedesktop entry and an icon, so the viewer
appears in the application menu with its own icon:

- Shared sources: `assets/linux/mikrotik-rif.desktop` and
  `assets/linux/mikrotik-rif-512.png` (a 512 px copy of `assets/icon/icon.png`,
  shipped because hicolor's `index.theme` has no 1024 directory).
- `.deb` and `.AppImage`: cargo-packager renders
  `usr/share/applications/mikrotik-rif.desktop` from the `deb.desktop-template`
  configured in `Cargo.toml`, and copies every `.png` listed in `icons` to
  `usr/share/icons/hicolor/<W>x<H>/apps/` (verified in
  `crates/packager/src/package/deb/mod.rs`; AppImage reuses that same code path).
- `.rpm`: `cargo-generate-rpm` has no freedesktop knowledge, so the desktop file
  and the 512 px icon are installed explicitly through the
  `[package.metadata.generate-rpm]` assets.
- `Icon=mikrotik-rif` in the desktop file matches the installed icon name, so
  the lookup resolves whichever hicolor size directory holds it.

Modern Fedora/RHEL run `update-desktop-database` and `gtk-update-icon-cache`
through RPM file triggers, so no scriptlets are needed.

## Verified prerequisites and their sources

These were verified against current upstream sources in September 2026.

| Topic | Finding | Source |
| --- | --- | --- |
| cargo-packager version and CLI | Latest `0.11.8`. It is a cargo subcommand (`cargo install cargo-packager --locked`); config comes from `[package.metadata.packager]`. | crates.io API, README in `crabnebula-dev/cargo-packager` |
| Does cargo-packager build the app? | No. README: "By default, the packager doesn't build your application." A prior `cargo build --release` is required, and `--release` tells the packager to read `target/release`. | cargo-packager README |
| WiX / MSI | cargo-packager downloads and SHA-256-verifies its own WiX 3.11.2 binaries from `wixtoolset/wix3`; it does not rely on a system install. Additionally, `windows-latest` (Windows Server 2025) already ships WiX Toolset 3.14.1.8722. | `crates/packager/src/package/wix/mod.rs`; `actions/runner-images` `Windows2025-Readme.md` |
| NSIS | NSIS is **not** preinstalled on `windows-latest`, but cargo-packager downloads and SHA-1-verifies NSIS 3.09 itself. No `choco`/`winget` step is needed. | `crates/packager/src/package/nsis/mod.rs`; `Windows2025-Readme.md` |
| AppImage | cargo-packager downloads `linuxdeploy`, `AppRun` and `linuxdeploy-plugin-appimage` at package time and runs them with `--appimage-extract-and-run`. Since 0.11.7 the plugin no longer requires FUSE on the host. `patchelf` 0.18.0 is already present on `ubuntu-latest`. | `crates/packager/src/package/appimage/*`; cargo-packager CHANGELOG 0.11.7; `Ubuntu2404-Readme.md` |
| RPM | cargo-packager has no RPM format (enum: `App`, `Dmg`, `Wix`, `Nsis`, `Deb`, `AppImage`, `Pacman`). `cargo-generate-rpm` (current `0.21.0`) is used instead. It does **not** build the binary; run it after `cargo build --release`. | `crates/utils/src/lib.rs`; cargo-generate-rpm 0.21.0 README |
| RPM metadata requirement | `cargo-generate-rpm` 0.21.0 requires a `[package.metadata.generate-rpm]` table with `assets` (`Config::new_from_manifest` returns `ConfigError::Missing("package.metadata.generate-rpm")` otherwise). The table lives in `Cargo.toml`, so the workflow only runs `cargo generate-rpm --output dist`. | `src/config/metadata.rs` (crate 0.21.0) |
| arm64 runner availability | `ubuntu-24.04-arm`, `ubuntu-22.04-arm` and `windows-11-arm` are **general availability** standard GitHub-hosted runners and are usable in **private** repositories (2 vCPU private / 4 vCPU public; usage counts towards plan minutes). They are also listed in the official runner reference alongside the x64 labels. | GitHub Changelog 2026-01-29 "arm64 standard runners are now available in private repositories"; GitHub Changelog 2026-08-20 "Linux and Windows arm64 standard hosted runners are now supported in all repositories"; docs.github.com "GitHub-hosted runners reference" |
| arm64 runner images | `Ubuntu 24.04` arm64 ships `patchelf` 0.18.0, `dpkg`/`dpkg-dev` and Rust 1.98.1. `Windows 11` arm64 ships **NSIS 3.10** (WiX is not listed) and Rust 1.98.1; Windows 11 arm64 emulates x86/x64, which is what cargo-packager's downloaded WiX 3.11.2 and NSIS 3.09 toolchains need. | `actions/runner-images` `Ubuntu2404-Arm64-Readme.md`, `Windows11-Arm64-Readme.md` |
| arm64 AppImage tooling | cargo-packager resolves the host arch and downloads `linuxdeploy-aarch64.AppImage`, `AppRun-aarch64` and `linuxdeploy-plugin-appimage-aarch64.AppImage`; all three assets exist upstream, so the arm64 AppImage build does not depend on FUSE or a system `linuxdeploy`. | cargo-packager 0.11.8 `src/package/appimage/mod.rs`; upstream release assets in `tauri-apps/binary-releases` (tags `linuxdeploy`, `apprun-old`) and `linuxdeploy/linuxdeploy-plugin-appimage` (tag `continuous`) |
| Windows arm64: native, not cross-compilation | The workflow uses the native `windows-11-arm` runner. For reference, `cargo packager --target <triple>` is supported (the CLI `--target` sets `Config::target_triple`, and the default binary directory becomes `target/<triple>/release`), but cargo-packager never builds: a non-HOST target would require a prior `cargo build --target aarch64-pc-windows-msvc` with the arm64 MSVC toolchain (or `cargo-xwin`). Native packaging avoids that entirely. | cargo-packager 0.11.8 `src/cli/mod.rs`, `src/config/mod.rs`, `src/cli/config.rs`; README ("the packager doesn't build your application") |
| Linux arm64: native required | Cross-compiling eframe/wgpu from an x86_64 runner is impractical because the dependency graph links native X11/Wayland, Vulkan and winit/rfd code; the native `ubuntu-24.04-arm` runner is used instead. | engineering assessment based on `Cargo.lock` (wayland/x11/vulkan/ash, `xkbcommon-dl`) and the eframe Linux build requirements |
| Linux dev libraries | eframe 0.36 documents `libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev` (+ `libssl-dev`). `rfd` 0.17.2 defaults to `xdg-portal` + `wayland`, so **GTK3 is not required**. `Cargo.lock` contains no `openssl-sys`, so `libssl-dev` is not required. `libudev-dev`/`libasound2-dev` are only needed for gamepad/audio features that are not enabled. | `emilk/egui` eframe README; crates.io feature list for `rfd` 0.17.0/0.17.2; `Cargo.lock` |

No official `cargo-packager` GitHub Action could be found: `cargo-packager/action`
returns HTTP 404, no action repository exists under `crabnebula-dev`, and the
main repository has no root `action.yml`. The release workflow therefore invokes
the official CLI directly.

## Cutting a release

1. Ensure `main` is green in CI.
2. Update the version in `Cargo.toml` (owned by the maintainer) and merge.
3. Create and push an annotated tag:
   ```bash
   git tag -a v0.1.0 -m "v0.1.0"
   git push origin v0.1.0
   ```
4. The Release workflow builds all installers, computes `SHA256SUMS.txt`, and
   creates the GitHub Release with `generate_release_notes: true`.

To build installers without publishing (dry run), run the Release workflow
manually via `workflow_dispatch`. Artifacts are uploaded, but the `release` job
is skipped.

## Verification

After a run, verify locally from the downloaded artifacts:

```bash
sha256sum -c SHA256SUMS.txt
```

On Linux (pick the architecture you need; both are attached to the release):

```bash
# Debian/Ubuntu
sudo apt install ./mikrotik-rif_*_amd64.deb      # x86_64
sudo apt install ./mikrotik-rif_*_arm64.deb      # arm64
# AppImage
chmod +x mikrotik-rif_*_x86_64.AppImage && ./mikrotik-rif_*_x86_64.AppImage
# Fedora/RHEL
sudo dnf install ./mikrotik-rif-*.x86_64.rpm     # x86_64
sudo dnf install ./mikrotik-rif-*.aarch64.rpm    # arm64
```

On macOS, after copying the app to `/Applications`:

```bash
xattr -dr com.apple.quarantine "/Applications/MikroTik RIF Viewer.app"
```

On Windows, if SmartScreen blocks the installer: **More info → Run anyway**.

## Rollback

- A bad release: delete the GitHub Release/tag, fix the issue, and push a new
  tag. Tags are immutable references, so do not force-push an existing tag.
- A bad CI change: revert the commit under `.github/workflows/`; the workflows
  are the only CI state and contain no secrets.
- Artifacts are retained for 7 days, so re-running a failed matrix job is
  possible while the run is available.

## Residual uncertainty (only provable on a real run)

- cargo-packager's WiX/NSIS/linuxdeploy downloads depend on third-party GitHub
  release assets that are pinned by hash in cargo-packager but could be
  unavailable or rate-limited.
- Windows arm64 is the main unknown: cargo-packager downloads **x86** WiX and
  NSIS toolchains and runs them under Windows 11 arm64 emulation. This should
  work (the image also ships NSIS 3.10, and x86 emulation is supported), but
  the `.msi`/`-setup.exe` builds on `windows-11-arm` are only proven on a real
  run.
- AppImage packaging requires the app icon to be square; `assets/icon/icon.png`
  is assumed square. The arm64 build additionally depends on the upstream
  `linuxdeploy-aarch64`/`AppRun-aarch64` assets remaining available.
- arm64 standard runners are 2 vCPU in private repositories, so the arm64 jobs
  are slower and consume plan minutes (they are free-minute eligible).
- The exact output filenames (`mikrotik-rif_*`, `MikroTik-RIF-Viewer.app.zip`)
  are consumed through globs, so harmless naming changes will not break the
  workflow.
- macOS `.app`/`.dmg` Gatekeeper behavior is only observable on a real macOS
  client.
