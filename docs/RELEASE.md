# Release engineering

This document describes how `mikrotik-rif` is built and released by GitHub
Actions. The workflows live in `.github/workflows/`.

## Workflows

There are three workflows. `build.yml` owns the packaging matrix; `release.yml`
only orchestrates it; `ci.yml` is pure test.

| Workflow | File | Trigger | Purpose |
| --- | --- | --- | --- |
| CI | `.github/workflows/ci.yml` | `push` to `main`, `pull_request`, `workflow_dispatch` | Format, lint and test on Linux, macOS and Windows. It does not build release binaries, package anything, or upload artifacts. |
| Build | `.github/workflows/build.yml` | `workflow_dispatch`, `workflow_call` | Build and package the native installers for all five runner/architecture combinations and upload them as workflow artifacts. It never publishes a release. |
| Release | `.github/workflows/release.yml` | `push` of tags matching `v*`, `workflow_dispatch` | Orchestration only: it calls `build.yml` (job `build`), then — for `v*` tags and only then — downloads the artifacts, computes checksums and publishes a GitHub Release. |

### Which workflow runs when

- **Push to `main` / pull request** → `ci.yml` only.
- **Manual `workflow_dispatch` of `build.yml`** → packages every platform and
  uploads the artifacts to that run. Nothing is published.
- **Manual `workflow_dispatch` of `release.yml`** → calls `build.yml`, uploads
  the same artifacts, and skips the `release` job because `github.ref` is a
  branch, not a `v*` tag.
- **Push of a `v*` tag** → `release.yml` calls `build.yml`, then the `release`
  job runs and publishes the GitHub Release.

**Only a `v*` tag publishes a release.** The `release` job is gated on
`startsWith(github.ref, 'refs/tags/v')`; every other entry point stops after
uploading artifacts.

`release.yml` calls `build.yml` with `uses: ./.github/workflows/build.yml`. A
reusable workflow invoked via `workflow_call` runs inside the caller's workflow
run (the jobs share `github.run_id`), so the artifacts uploaded by `build.yml`
belong to the caller's run and the `release` job fetches them with
`actions/download-artifact`, whose default `run-id` is `${{ github.run_id }}`.
This is the standard way to share build artifacts across a `workflow_call`
boundary; no extra inputs or `run-id` overrides are needed. Permissions can only
be maintained or reduced across a reusable-workflow call, so `release.yml`
grants `contents: write` at the workflow level but caps the `build` call at
`contents: read`.

CI installs the Linux development packages eframe/winit/wgpu need. The build
workflow builds the application once per runner (`cargo build --release --locked`)
and lets cargo-packager consume that binary; it does **not** compile the app
again. Each architecture is built on its own **native** arm64/x64 runner (no
cross-compilation).

## Artifacts produced per runner and architecture

Every artifact is published under one scheme,
`mikrotik-rif_<version>_<arch>.<ext>` (or `..._<arch>-setup.exe` on Windows),
with `<arch>` in `{amd64, arm64}`. The tools name their output their own way —
cargo-packager after the binary, cargo-generate-rpm after the RPM NEVRA, and
the macOS disk image after the product — so `build.yml` renames everything to
this scheme in its "Normalise release asset names" step.

| Runner | Architecture | Tool | Output format (published name uses `amd64` / `arm64`) |
| --- | --- | --- | --- |
| `windows-latest` | x64 | cargo-packager | NSIS `-setup.exe` |
| `windows-11-arm` | arm64 | cargo-packager | NSIS `-setup.exe` |
| `macos-latest` | arm64 (Apple Silicon) | cargo-packager | `.dmg` (the `.app` bundle is built as an intermediate step and imaged into the disk image) |
| `ubuntu-latest` | x64 | cargo-packager | `.deb` |
| `ubuntu-24.04-arm` | arm64 | cargo-packager | `.deb` |
| `ubuntu-latest` | x64 | cargo-generate-rpm | `.rpm` |
| `ubuntu-24.04-arm` | arm64 | cargo-generate-rpm | `.rpm` |

The architecture is part of every published name, so the amd64 and arm64
artifacts never collide when the release job merges them into a single `dist/`
directory. Note that the `.rpm` **file** is renamed but keeps its internal
NEVRA (`mikrotik-rif-<version>-1.<arch>`); only the download name is uniform.

All builds are **unsigned**. No signing certificates or secrets are used.

## Updater asset contract

The in-app updater (`src/update.rs`, the only network code in the product)
polls `https://api.github.com/repos/balakar94/mikrotik-rif/releases/latest`,
compares `tag_name` (`vX.Y.Z`) against its own version, and picks the installer
for the current platform with the matching rules documented in that module
(NSIS `-setup.exe` with `_amd64`/`_arm64` on Windows, `.dmg` on Apple Silicon,
and `.deb` → `.rpm` → `.AppImage` with `_amd64`/`_arm64` on Linux, preferring
native packages).

**The file names above are a contract with the updater.** If `build.yml` ever
renames an artifact, the table in `src/update.rs` (and its tests) must be
updated in the same change; otherwise the updater finds no match and falls
back to opening the release page in the browser. The same holds for
`SHA256SUMS.txt`: the release job must keep attaching it, because the updater
deletes any download whose SHA-256 does not match its entry and never executes
an unverified file. Packaged file names must also be **free of spaces and
unusual characters** — GitHub Releases rewrites them on upload while
`SHA256SUMS.txt` is generated from the local names, and a mismatch breaks both
`sha256sum -c` and the updater's checksum lookup (it finds the asset by its
published name); the normalisation step in `build.yml` enforces that.

A fully silent handoff exists on Windows (the
installer is launched and the app exits) and for Linux AppImages (the running
image is replaced atomically and relaunched); on macOS and for native Linux
packages the verified file is opened from `~/Downloads` so the user — or the
system software manager — finishes the install. macOS deliberately keeps the
open-the-`.dmg` flow: the project ships unsigned builds and will not
self-replace the `/Applications` bundle, because Gatekeeper would treat every
replaced bundle as a new app.

### Conditional checks and optional signing

The check sends `If-None-Match` with the ETag of the previous response
(persisted as `update.etag`); an unchanged `releases/latest` answers `304`, and
GitHub does not count those against the 60-requests-per-hour unauthenticated
limit. This matters because the automatic check runs on every launch.

Releases can additionally be signed with minisign. It is opt-in and off by
default:

- generate an **unencrypted** key pair with `minisign -G -W` (the `-W` is
  required: CI cannot answer the interactive password prompt) and store the
  contents of `minisign.key` in the `MINISIGN_SECRET_KEY` repository secret —
  never commit or share it;
- put the public key in the `MIKROTIK_RIF_MINISIGN_PUBKEY` repository
  **variable** (not a secret). Either the whole `minisign.pub` file or just its
  second line (the base64 key) works. `build.yml` bakes it into the binary; with
  it embedded, the updater requires a valid `SHA256SUMS.txt.minisig` and refuses
  any download that lacks one;
- `release.yml` then signs `SHA256SUMS.txt` with `minisign -S -W` and uploads
  the detached signature.

Configure the variable and the secret **together**: a public key without a
signing step makes every update fail closed, while a signing step without the
public key is inert. The verification path is unit-tested against a minisign
fixture; the CI signing branch has not been exercised on a real release.

## Release integrity gates

`release.yml` runs three checks around the publish job (`v*` tags only), so a
release cannot publish assets that disagree with its tag or with its own
checksums:

- **Tag ↔ version**: `GITHUB_REF_NAME` must equal `v<version>` read from
  `Cargo.toml`, because cargo-packager bakes that version into every normalised
  asset name.
- **Manifest ↔ files (pre-publish)**: `SHA256SUMS.txt` must list exactly the
  files in `dist/`, and no name may contain whitespace — GitHub rewrites spaces
  on upload, which is what broke the v0.2.0 `.dmg`
  (`MikroTik RIF Viewer_0.2.0_aarch64.dmg` was published as
  `MikroTik.RIF.Viewer_0.2.0_aarch64.dmg`, so `sha256sum -c` no longer matched).
- **Manifest ↔ published assets (post-publish)**: `gh release view` re-reads the
  real asset names. This runs after the release exists, so a failure there means
  "fix the names", not "nothing was published".

The updater in `src/update.rs` accepts asset downloads only from `github.com`,
`api.github.com` and GitHub's object-store hosts (`objects...` and the current
`release-assets.githubusercontent.com`); every redirect hop is re-validated, so
a new host must be added there deliberately or downloads fail closed.

## Linux desktop integration

Every Linux package installs a freedesktop entry, an icon and a shared-MIME
definition, so the viewer appears in the application menu with its own icon
and double-clicking a `.rif` file opens it in the viewer:

- Shared sources: `assets/linux/mikrotik-rif.desktop`,
  `assets/linux/mikrotik-rif-mime.xml` (defines
  `application/x-mikrotik-rif` with a `*.rif` glob) and
  `assets/linux/mikrotik-rif-512.png` (a 512 px copy of `assets/icon/icon.png`,
  shipped because hicolor's `index.theme` has no 1024 directory).
- `.deb` and `.AppImage`: cargo-packager renders
  `usr/share/applications/mikrotik-rif.desktop` from the `deb.desktop-template`
  configured in `Cargo.toml`, and copies every `.png` listed in `icons` to
  `usr/share/icons/hicolor/<W>x<H>/apps/` (verified in
  `crates/packager/src/package/deb/mod.rs`; AppImage reuses that same code path
  via `deb::generate_data`). The MIME XML travels through the `deb.files` and
  `appimage.files` maps to `/usr/share/mime/packages/mikrotik-rif.xml`.
- `.rpm`: `cargo-generate-rpm` has no freedesktop knowledge, so the desktop file,
  the 512 px icon and the MIME XML are installed explicitly through the
  `[package.metadata.generate-rpm]` assets.
- `Icon=mikrotik-rif` in the desktop file matches the installed icon name, so
  the lookup resolves whichever hicolor size directory holds it.
- `MimeType=application/x-mikrotik-rif;` in the desktop file advertises the
  association; the XML defines the type itself. Both halves are required for a
  new (previously unknown) type: without the XML, `MimeType=` points at a type
  no tool knows. `Exec=mikrotik-rif %F` passes the capture path as `argv[1]`,
  which the app already reads. The desktop template is deliberately
  placeholder-free (renders to itself), so the `MimeType` line is hardcoded to
  match the declarative `mime-type` in the
  `[[package.metadata.packager.file-associations]]` table — a placeholder-free
  template would otherwise silently drop the generated `{{mime_type}}` value
  (verified: `generate_desktop_file` in `crates/packager/src/package/deb/`
  renders the custom template with the same `exec_arg`/`mime_type` context,
  and cargo-packager ships no shared-mime-info handling anywhere in `src/`).

Modern Fedora/RHEL run `update-desktop-database` and `gtk-update-icon-cache`
through RPM file triggers, so no scriptlets are needed; the same holds for the
MIME database (`shared-mime-info` packaging hooks process
`/usr/share/mime/packages/` trigger-based on Debian and RPM families alike).

## macOS file association

The same `[[package.metadata.packager.file-associations]]` table becomes a
`CFBundleDocumentTypes` entry in the `.app` `Info.plist`
(`crates/packager/src/package/app/mod.rs`): `CFBundleTypeExtensions = ["rif"]`,
`CFBundleTypeName` from `name`, `CFBundleTypeRole = Viewer` from
`role = "viewer"` (read-only fits this viewer; the default would be `Editor`).
Finder double-click then opens the capture in the viewer. No context-menu or
`CFBundleURLTypes` entries are added.

## macOS disk image background

The `.dmg` opens a 660x400 drag-to-`/Applications` window with product
identity: `assets/macos/dmg-background.png` (dark gradient echoing
`assets/icon/icon.svg`, two drop-zone rings, sky arrow; no text, so it is
localization-free, and no baked-in icons — Finder draws the `.app` icon and
the `/Applications` symlink on top of the rings). It is wired through the
`[package.metadata.packager.dmg]` table in `Cargo.toml` (`background`,
`window-size`, `app-position`, `application-folder-position` — the exact key
spellings accepted by cargo-packager 0.11.8; notably `app-folder-position`
and `window-position` do not exist there). The background path is resolved
relative to the repo root, where `cargo packager` runs in CI. The rendered
window can only be eyeballed on a macOS run (see residual uncertainty).

### Why CI-built `.dmg`s used to ship without the background

The `v0.2.0` `.dmg` embedded the correct 660x400 PNG (verified by mounting the
published image: `.background/dmg-background.png` was byte-identical to
`assets/macos/dmg-background.png`), yet Finder opened a plain window. The cause
was not a missing change — the config and the asset were both ancestors of the
`v0.2.0` tag — but cargo-packager 0.11.8 itself:

```rust
// crates/packager/src/package/dmg/mod.rs
if let Some(value) = std::env::var_os("CI") {
    if value == "true" {
        bundle_dmg_cmd.arg("--skip-jenkins");
    }
}
```

GitHub Actions exports `CI=true`, so the DMG was built with create-dmg's
`--skip-jenkins`. That flag skips the Finder AppleScript that is the only thing
which writes the mounted volume's `.DS_Store`; without it Finder has no record
of the background or the icon layout. The image contains the PNG but no
`.DS_Store`, so the wallpaper never appears.

The packaging step in `build.yml` now overrides `CI` to `false` on macOS only
(exact-string compare against `"true"` makes this reliable; Linux/Windows keep
`CI=true` and their packaging paths never read it), and a following macOS-only
step mounts the fresh `.dmg` and fails the build unless a root `.DS_Store`
exists. `create-dmg`'s AppleScript waits on Finder without its own timeout, so
the packaging step carries a 30-minute `timeout-minutes` bound. Rollback is
deleting the `env:` override and the verification step; the `.dmg`s then lose
the background again and the verification step fails.

### Why the `.dmg` has no click-through licence agreement (EULA)

`[package.metadata.packager]` in `Cargo.toml` deliberately has **no
`license-file`**. In cargo-packager 0.11.8 that key is consumed by three
formats, and one of them cannot work headless:

- **DMG** — `crates/packager/src/package/dmg/mod.rs` passes it to create-dmg as
  `--eula`, which applies it with `hdiutil udifrez` (`LPic`/`STR#`/`TEXT`
  resources). Mounting the image then prints the licence and waits for "yes";
  a non-interactive `hdiutil attach` (CI, install scripts, `</dev/null`) reads
  EOF and aborts with `hdiutil: attach canceled`. That failed the macOS
  packaging job in run `34793505291` at the `.DS_Store` verification step, and
  `hdiutil attach -acceptlicense` does not exist on current macOS, so there is
  no supported way to accept it non-interactively.
- **NSIS** — the same key inserts `MUI_PAGE_LICENSE` into the Windows
  installers. With the key unset, no licence page is shown.
- **WiX** — unused here (both Windows jobs are NSIS-only).

MIT is a grant, not a contract that needs assent: its only condition is that
the copyright/permission notice travels with copies, which the `resources`
list (and the `deb.files` / `appimage.files` / `generate-rpm` maps) already
guarantees — `LICENSE` and `THIRD-PARTY-NOTICES.md` sit inside the `.app`, the
`.dmg`, the `-setup.exe`, the `.deb`, the `.rpm` and the AppImage. The EULA
bought nothing and broke unattended mounts, so it is off. The macOS
verification step in `build.yml` still mounts the image with no TTY, which
doubles as the regression guard: an explicit `hdiutil udifderez` check reports
an embedded agreement with the real cause, and re-adding `license-file` makes
the attach fail again.

## Windows file association

The same table is rendered once per extension into the NSIS installer
(`crates/packager/src/package/nsis/installer.nsi`): the `APP_ASSOCIATE` macro
writes `HKCR\.rif` plus a `shell\open\command` of
`"<INSTDIR>\mikrotik-rif.exe" "%1"` — the default double-click `open` verb
only. The literal `"Open with ${PRODUCTNAME}"` in that template line is the
display text of that default verb, not an extra entry: no
`APP_ASSOCIATE_ADDVERB` verb is emitted, so no right-click menu item is created
(per explicit user constraint), and `description` only feeds the Explorer
"Type" column. Both Windows jobs (`x64`, `arm64`) are NSIS-only, so both get
the association.

Known upstream defect (cargo-packager 0.11.8): the *uninstall* section of
`installer.nsi` iterates `{{#each association.ext}}` while the struct field is
`extensions` (the install section was fixed to `association.extensions` in
commit `c34de36`, the uninstall section was not). The generated uninstaller
therefore renders an empty loop and leaves the `.rif` registry keys behind.
Documented here instead of worked around: a custom NSIS template would fork
upstream installer logic for a one-word fix.

## Licence files in the packages

The application is MIT, but the three fonts it embeds are under the SIL Open
Font License 1.1, whose condition 2 requires the copyright notice and the
licence text to accompany every distributed copy of the font — including the
copies inside a compiled binary. `icons`, `desktop-template` and the release
attachments alone do not satisfy that, so:

- the repository carries one OFL text per bundled font
  (`assets/fonts/OFL-Inter.txt`, `OFL-JetBrainsMono.txt`, `OFL-NotoSansSC.txt`);
- `.deb` and `.AppImage` install all three plus `LICENSE` under
  `/usr/share/licenses/mikrotik-rif/`, through the `deb.files` and
  `appimage.files` maps in `Cargo.toml` (cargo-packager strips a leading `/`
  from the destination and copies the file to that path inside the package);
- `.rpm` does the same through the `[package.metadata.generate-rpm]` assets;
- macOS and Windows have no such file map, so the same four texts travel
  through the top-level `resources` list: cargo-packager copies them into
  `Contents/Resources` in the `.app` (and therefore the `.dmg`) and next to the
  executable on Windows. On `.deb` this also duplicates them under
  `usr/lib/<binary>/licenses/`, which is harmless. No click-through EULA is
  embedded in the `.dmg` and NSIS shows no licence page, because `license-file`
  is deliberately unset (see above); this resource copy is what satisfies MIT's
  notice condition;
- every GitHub Release attaches only the installers plus `SHA256SUMS.txt`. The
  texts above still travel *inside* every package (which is what the OFL
  requires); they are just no longer attached as loose files on the release
  page.

### Third-party notices and egui's default fonts

`FontDefinitions::default()` means egui also embeds four fonts of its own
(Hack, Noto Emoji, Ubuntu Light and emoji-icon-font). Their licence texts are
vendored from the `epaint_default_fonts` crate into `assets/fonts/egui/`
(`MIT-Hack.txt`, `OFL-NotoEmoji.txt`, `UFL-Ubuntu-Light.txt`,
`MIT-emoji-icon-font.txt`) and ship alongside the app exactly like the bundled
fonts.

The notices of the Rust dependency graph (444 of the 473 `Cargo.lock`
packages are reachable and compiled; the rest are feature/target-gated and
never linked) are generated with `cargo-about` into `THIRD-PARTY-NOTICES.md`:

```sh
cargo install cargo-about --locked --version 0.9.2 --features cli
cargo about generate about.hbs --output-file THIRD-PARTY-NOTICES.md --fail --frozen
```

`about.toml` keeps the accepted-licence list in sync with `deny.toml`, and the
`deny` CI job regenerates the file and fails if it drifts from `Cargo.lock`.
All eight texts (the four bundled-font licences, `LICENSE`, the four egui-font
licences and `THIRD-PARTY-NOTICES.md`) travel inside `.deb`, `.rpm`,
`.AppImage`, the `.app`/`.dmg` and the Windows installer.


## Verified prerequisites and their sources

These were verified against current upstream sources in September 2026.

| Topic | Finding | Source |
| --- | --- | --- |
| cargo-packager version and CLI | Latest `0.11.8`. It is a cargo subcommand (`cargo install cargo-packager --locked`); config comes from `[package.metadata.packager]`. | crates.io API, README in `crabnebula-dev/cargo-packager` |
| Does cargo-packager build the app? | No. README: "By default, the packager doesn't build your application." A prior `cargo build --release` is required, and `--release` tells the packager to read `target/release`. | cargo-packager README |
| WiX / MSI (removed) | MSI support was dropped: both Windows jobs ship NSIS-only `-setup.exe`. cargo-packager's WiX 3.11.2 `candle.exe` (x86 .NET Framework 3.5) died on `windows-11-arm` in the v0.1.0 run, and the unsigned MSI bought nothing over the unsigned NSIS installer for manual installs. No WiX tooling is downloaded anymore. | `crates/packager/src/package/wix/mod.rs`; observed CI run 34709776279 |
| NSIS | NSIS is **not** preinstalled on `windows-latest`, but cargo-packager downloads and SHA-1-verifies NSIS 3.09 itself. No `choco`/`winget` step is needed. | `crates/packager/src/package/nsis/mod.rs`; `Windows2025-Readme.md` |
| AppImage | cargo-packager downloads `linuxdeploy`, `AppRun` and `linuxdeploy-plugin-appimage` at package time and runs them with `--appimage-extract-and-run`. Since 0.11.7 the plugin no longer requires FUSE on the host. `patchelf` 0.18.0 is already present on `ubuntu-latest`. | `crates/packager/src/package/appimage/*`; cargo-packager CHANGELOG 0.11.7; `Ubuntu2404-Readme.md` |
| RPM | cargo-packager has no RPM format (enum: `App`, `Dmg`, `Wix`, `Nsis`, `Deb`, `AppImage`, `Pacman`). `cargo-generate-rpm` (current `0.21.0`) is used instead. It does **not** build the binary; run it after `cargo build --release`. | `crates/utils/src/lib.rs`; cargo-generate-rpm 0.21.0 README |
| RPM metadata requirement | `cargo-generate-rpm` 0.21.0 requires a `[package.metadata.generate-rpm]` table with `assets` (`Config::new_from_manifest` returns `ConfigError::Missing("package.metadata.generate-rpm")` otherwise). The table lives in `Cargo.toml`, so the workflow only runs `cargo generate-rpm --output dist`. | `src/config/metadata.rs` (crate 0.21.0) |
| File-association config keys | Top-level `Config::file_associations: Option<Vec<FileAssociation>>` accepts `fileAssociations` (canonical camelCase) plus `file-associations` / `file_associations` aliases; `FileAssociation` requires `extensions` (bare, e.g. `rif` — the NSIS template prepends the dot) with `mimeType` (alias `mime-type`, Linux-only), `description` (Windows-only, Explorer Type column), `name` (macOS `CFBundleTypeName`, Windows file-class) and `role` (default `"editor"`). `Cargo.toml` uses the kebab-case aliases to match the rest of the file. | cargo-packager 0.11.8 `src/config/mod.rs`, `schema.json`, `src/cli/config.rs` (Cargo.toml → `Config` via serde_json) |
| NSIS double-click, no context menu | `installer.nsi` expands one `APP_ASSOCIATE` per extension: `HKCR\.rif` → file-class with `DefaultIcon` and a single `shell\open\command = "<INSTDIR>\*.exe" "%1"`. That is the default double-click verb; the `"Open with …"` literal is that verb's display text, and no `APP_ASSOCIATE_ADDVERB` is emitted, so no right-click entry is created. Defect: the uninstall section still iterates `association.ext` (stale; install side was fixed to `association.extensions` in `c34de36`), so uninstall leaves the keys behind. | cargo-packager 0.11.8 `src/package/nsis/installer.nsi`, `src/package/nsis/FileAssociation.nsh`, CHANGELOG (`c34de36`) |
| macOS document types | `file_associations` becomes `CFBundleDocumentTypes` (`CFBundleTypeExtensions`, `CFBundleTypeName`, `CFBundleTypeRole` via `Display`: `Viewer`/`Editor`/…). `role = "viewer"` is used because the app only reads captures. | cargo-packager 0.11.8 `src/package/app/mod.rs` |
| Linux MIME plumbing gap | `generate_desktop_file` derives `exec_arg = "%F"` and `mime_type = join(";")` from `file_associations` and renders the *custom* desktop template with the same context — but only if the template uses `{{exec_arg}}`/`{{mime_type}}`. Ours is placeholder-free, so `MimeType=` is hardcoded to match, and cargo-packager ships no shared-mime-info support at all (`MimeType=` is the only `mime` reference in `src/`), hence `assets/linux/mikrotik-rif-mime.xml` installed via the `deb.files`/`appimage.files` maps and the `[package.metadata.generate-rpm]` assets. | cargo-packager 0.11.8 `src/package/deb/mod.rs`, `src/package/deb/main.desktop`; freedesktop shared-mime-info and desktop-entry specs |
| arm64 runner availability | `ubuntu-24.04-arm`, `ubuntu-22.04-arm` and `windows-11-arm` are **general availability** standard GitHub-hosted runners and are usable in **private** repositories (2 vCPU private / 4 vCPU public; usage counts towards plan minutes). They are also listed in the official runner reference alongside the x64 labels. | GitHub Changelog 2026-01-29 "arm64 standard runners are now available in private repositories"; GitHub Changelog 2026-08-20 "Linux and Windows arm64 standard hosted runners are now supported in all repositories"; docs.github.com "GitHub-hosted runners reference" |
| arm64 runner images | `Ubuntu 24.04` arm64 ships `patchelf` 0.18.0, `dpkg`/`dpkg-dev` and Rust 1.98.1. `Windows 11` arm64 ships **NSIS 3.10** (WiX is not listed) and Rust 1.98.1, and emulates x86/x64. Emulation is enough for NSIS 3.09 (native Win32) but **not** for WiX 3.11.2's `candle.exe`, which failed in the v0.1.0 run — one more reason both Windows jobs are NSIS-only. | `actions/runner-images` `Ubuntu2404-Arm64-Readme.md`, `Windows11-Arm64-Readme.md`; observed CI run 34709776279 |
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
   git tag -a v0.3.0 -m "v0.3.0"
   git push origin v0.3.0
   ```
4. The Release workflow calls `build.yml`, computes `SHA256SUMS.txt` from the
   merged artifacts, and creates the GitHub Release with
   `generate_release_notes: true`.

To build installers without publishing (dry run), run the **Build** workflow
directly via `workflow_dispatch`; the installers are uploaded as artifacts on
that run page. Running **Release** manually via `workflow_dispatch` also builds
them (through the reusable call) but skips the `release` job, since the ref is
not a `v*` tag.

## Verification

After a run, verify locally from the downloaded artifacts:

```bash
sha256sum -c SHA256SUMS.txt
```

On Linux (pick the architecture you need; both are attached to the release):

```bash
# Debian/Ubuntu
sudo apt install ./mikrotik-rif_<version>_amd64.deb    # amd64
sudo apt install ./mikrotik-rif_<version>_arm64.deb    # arm64
# AppImage
chmod +x mikrotik-rif_<version>_amd64.AppImage && ./mikrotik-rif_<version>_amd64.AppImage
# Fedora/RHEL
sudo dnf install ./mikrotik-rif_<version>_amd64.rpm    # amd64
sudo dnf install ./mikrotik-rif_<version>_arm64.rpm    # arm64
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
  are the only CI state and contain no secrets. `build.yml` is the single
  packaging definition, so a packaging regression is fixed in one place.
- Artifacts are retained for 7 days, so re-running a failed matrix job is
  possible while the run is available. A `workflow_call` run uploads artifacts
  to the caller's run, so a manual `release.yml` dispatch also retains them for
  7 days.

## Residual uncertainty (only provable on a real run)

- cargo-packager's NSIS/linuxdeploy downloads depend on third-party GitHub
  release assets that are pinned by hash in cargo-packager but could be
  unavailable or rate-limited.
- Windows arm64: **observed failure (historical)**. cargo-packager's WiX 3.11.2
  `candle.exe` is an x86 .NET Framework 3.5 tool and dies on
  `windows-11-arm` with `Error running candle.exe` (exit 1), which skipped the
  publish job for v0.1.0. MSI support has since been removed: both Windows
  jobs package with NSIS only (a native Win32 tool).
- AppImage packaging requires the app icon to be square; `assets/icon/icon.png`
  is assumed square. The arm64 build additionally depends on the upstream
  `linuxdeploy-aarch64`/`AppRun-aarch64` assets remaining available.
- arm64 standard runners are 2 vCPU in private repositories, so the arm64 jobs
  are slower and consume plan minutes (they are free-minute eligible).
- The exact output filenames (`mikrotik-rif_*`)
  are consumed through globs, so harmless naming changes will not break the
  workflow.
- macOS `.app`/`.dmg` Gatekeeper behavior is only observable on a real macOS
  client.
- The `.dmg` background (`assets/macos/dmg-background.png`, 660x400) and its
  icon slots (`app-position`, `application-folder-position`) are only
  eyeballable on a mounted image from a macOS CI run: check that the window
  is 660x400, the background fills it without scaling artifacts, and the
  `.app` icon plus the `/Applications` symlink sit centred on the two rings
  with the arrow between them. The `CI=false` override is the only way to get
  the background on a CI-built `.dmg`, and it re-enables create-dmg's Finder
  AppleScript: GitHub-hosted macOS runners normally do have an Aqua session,
  but if Finder never writes `.DS_Store` the packaging step hangs until its
  30-minute `timeout-minutes` (bounded failure), and if `osascript` errors the
  step fails in seconds. Both cases are caught by the `.DS_Store` verification
  step before any artifact is uploaded; validate the pair with the
  **Packaging smoke** workflow on a macOS runner before tagging.
- macOS `.dmg` EULA: re-adding `license-file` to `[package.metadata.packager]`
  embeds a click-through agreement and breaks every headless mount. The
  `udifderez` check plus the TTY-less `hdiutil attach` in the `.DS_Store`
  verification step catch it before upload (see "Why the `.dmg` has no
  click-through licence agreement (EULA)").
- File associations are only provable on real installers: NSIS registry writes
  (and the leftover `.rif` keys on uninstall, see above), macOS Launch Services
  registration of `CFBundleDocumentTypes` (a quarantined or relocated `.app`
  may need opening once before Finder binds `.rif`), and Linux MIME resolution
  (`gio mime application/x-mikrotik-rif`, `xdg-mime query default …` after
  install). The AppImage carries the same desktop file and MIME XML inside its
  AppDir, but host-wide double-click binding additionally needs a host
  integrator (e.g. appimage-launcher); without one, only launching via the
  AppImage file itself is guaranteed.
- Reusable-workflow artifact sharing: `build.yml` runs as part of the caller's
  run, so `actions/download-artifact` in `release.yml` is expected to see all
  five artifacts via its default `run-id`. This has not been exercised on a real
  run here; if the artifacts were ever not visible, the fallback is to add an
  explicit `run-id: ${{ github.run_id }}`, which is the same value the action
  already defaults to for a same-repo `workflow_call`.
