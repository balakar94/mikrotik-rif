//! In-app updater: the only code in the product that opens a socket.
//!
//! Everything else in this product runs locally and never opens a socket. This
//! module owns the only exception: a poll of the GitHub Releases API plus the
//! download of a release installer. The rules are:
//!
//! * When enabled, an automatic check runs on every launch, on a background
//!   [`std::thread`], and never blocks the interface thread.
//! * The user can disable automatic checks and can always trigger a manual one.
//!   A manual check ignores a previously skipped release, so "Skip this
//!   version" keeps the automatic checks quiet while Settings can still
//!   surface the update on demand.
//! * Nothing is ever executed without a verified SHA-256 checksum.
//! * There is no telemetry and no context menu. A fully silent install exists
//!   on Windows (installer launch) and for Linux AppImages (in-place replace
//!   and relaunch); on macOS the handoff always leaves the final step to the
//!   user. Deliberately unsigned releases stay that way: macOS keeps the
//!   open-the-`.dmg` flow instead of a self-replace that Gatekeeper would
//!   question again after every update.
//!
//! ## Repository coordinates
//!
//! [`UPDATE_OWNER`] and [`UPDATE_REPO`] are the single source of truth for the
//! update endpoint. They mirror `package.repository` in `Cargo.toml`, the git
//! remote (`github.com/balakar94/mikrotik-rif`) and the CI badges in
//! `README.md`; keep the three in sync.
//!
//! ## Asset-name contract
//!
//! Release installer file names are produced by `.github/workflows/build.yml`
//! and consumed by [`select_asset`]. The names are a **contract**: if the
//! workflow ever renames an artifact, the table below (and this module's
//! tests) must be updated together, or the updater will fall back to opening
//! the release page in the browser.
//!
//! Every artifact is named `mikrotik-rif_<version>_<arch>.<ext>` (or
//! `..._<arch>-setup.exe` on Windows), with `<arch>` in `{amd64, arm64}`.
//!
//! | OS (`std::env::consts::OS`) | Arch (`std::env::consts::ARCH`) | Asset match (all conditions hold) |
//! | --- | --- | --- |
//! | `windows` | `x86_64` | name ends with `-setup.exe` and contains `_amd64` |
//! | `windows` | `aarch64` | name ends with `-setup.exe` and contains `_arm64` |
//! | `macos` | `aarch64` | name ends with `.dmg` (only Apple Silicon is shipped, and exactly one `.dmg` per release; a second one would need an architecture marker here) |
//! | `linux` | `x86_64` | `.deb` containing `_amd64`, else `.rpm` containing `_amd64`, else `.AppImage` containing `_amd64` |
//! | `linux` | `aarch64` | `.deb` containing `_arm64`, else `.rpm` containing `_arm64`, else `.AppImage` containing `_arm64` |
//! | anything else | anything else | no match — the updater opens the release page instead |
//!
//! Preference rationale on Linux: when the app itself runs from an AppImage
//! (the runtime exports `$APPIMAGE`), the portable `.AppImage` wins so the
//! update can replace it in place. Otherwise a native package (`.deb` first,
//! then `.rpm`) integrates with the system package manager, so it wins over
//! the portable `.AppImage` when several formats are attached to the same
//! release.
//!
//! ## Handoff per platform
//!
//! * Windows: the `-setup.exe` is saved to the system temporary directory and
//!   launched as a detached child process; the app then closes itself so the
//!   installer can replace it.
//! * macOS: the `.dmg` is saved to `~/Downloads` and opened, so the user can
//!   drag the app into `/Applications`. Deliberate: the project ships unsigned
//!   builds and will not self-replace the `/Applications` bundle, because
//!   Gatekeeper would treat every replaced bundle as a new app and demand the
//!   manual approval gesture again after each update.
//! * Linux: when running from an AppImage, the verified `.AppImage` replaces
//!   the running file atomically (staging copy plus rename in the same
//!   directory, then `chmod 0755`) and the app relaunches itself. Otherwise
//!   the chosen package is saved to `~/Downloads` and opened with the desktop
//!   handler (`xdg-open` via the [`open`] crate), so the software manager
//!   completes the installation.
//!
//! If no asset matches the current platform, or the checksum file is missing
//! the asset's entry, the updater never downloads an executable blindly: it
//! opens the release page in the browser and lets the user pick.
//!
//! ## Version rules
//!
//! Tags have the form `vX.Y.Z`. Comparison uses semantic versioning: a tag
//! with a pre-release suffix never counts as an update (the
//! `releases/latest` endpoint never points at a pre-release or a draft
//! anyway; the guard below exists so a hand-crafted tag cannot smuggle a
//! pre-release through).
//!
//! ## Error messages
//!
//! Messages shown in the update banner for failures are deliberately plain
//! technical English produced by [`UpdateError`], not Fluent identifiers: they
//! carry file names, URLs and OS codes that would not translate, and the six
//! Fluent keys used for the banner's *states* would then also have to grow one
//! key per error kind in all seven locales. Localization stops at the states;
//! errors stay technical and actionable.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest as _, Sha256};

/// GitHub account that owns the releases polled by the updater.
///
/// Centralized here; see the module documentation for why.
pub const UPDATE_OWNER: &str = "balakar94";
/// Repository whose releases are polled by the updater.
///
/// Centralized here; see the module documentation for why.
pub const UPDATE_REPO: &str = "mikrotik-rif";
/// Checksum file attached to every release by the release workflow.
pub const CHECKSUMS_FILE_NAME: &str = "SHA256SUMS.txt";
/// Detached minisign signature attached to the checksums file when the release
/// was signed (`SHA256SUMS.txt.minisig`).
pub const SIGNATURE_FILE_NAME: &str = "SHA256SUMS.txt.minisig";
/// Public key embedded at build time (`MIKROTIK_RIF_MINISIGN_PUBKEY`), or empty
/// when release signing is not configured.
pub const MINISIGN_PUBLIC_KEY: &str = env!("MIKROTIK_RIF_MINISIGN_PUBKEY");
/// Longest release-notes excerpt shown in the update banner, in characters.
pub const MAX_NOTES_CHARS: usize = 280;
/// `Accept` header sent to the GitHub API.
const ACCEPT_JSON: &str = "application/vnd.github+json";
/// Prefix every published release artifact shares; used to anchor asset
/// matching so an unrelated attachment cannot be mistaken for an installer.
const ASSET_PREFIX: &str = "mikrotik-rif_";
/// Network timeout for the small release-metadata request.
const METADATA_TIMEOUT: Duration = Duration::from_secs(25);
/// Network timeout for installer downloads (large files on slow links).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_mins(10);
/// Streaming buffer used while writing a download to disk.
const CHUNK_LEN: usize = 64 * 1024;
/// Hosts allowed for update traffic. Every URL (initial and per redirect hop)
/// must land on one of these, so a compromised API response cannot point the
/// downloader at an arbitrary host.
///
/// GitHub serves release assets through its object store and has migrated the
/// redirect target: `objects.githubusercontent.com` was the historical host,
/// while `release-assets.githubusercontent.com` is what the current edge
/// returns. Both are kept because the host a given client is redirected to
/// depends on GitHub's edge and can change over time.
const ALLOWED_HOSTS: [&str; 4] = [
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
/// Redirect hops followed manually; each hop is re-validated by [`check_url`].
const MAX_REDIRECTS: usize = 3;
/// Upper bound for the release-metadata JSON body (1 MiB).
const MAX_METADATA_BYTES: u64 = 1024 * 1024;
/// Upper bound for the `SHA256SUMS.txt` body (1 MiB).
const MAX_CHECKSUMS_BYTES: u64 = 1024 * 1024;
/// Upper bound for a downloaded installer (512 MiB).
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
/// Maximum number of assets accepted in a release before it is rejected.
const MAX_ASSETS: usize = 200;
/// Maximum length accepted for identifier-like JSON fields (tag, URL, name).
const MAX_FIELD_BYTES: usize = 8 * 1024;
/// Maximum length accepted for the release-notes body (only 280 characters are
/// ever displayed, but the JSON is parsed whole).
const MAX_NOTES_BYTES: usize = 256 * 1024;
/// Longest `ETag` value kept from a response or from storage (2 KiB).
const MAX_ETAG_BYTES: usize = 2048;

/// What went wrong while checking, downloading, or handing off an update.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// A URL was not `https://` on an allowed GitHub host.
    #[error("refused update URL: {0}")]
    InvalidUrl(String),
    /// The HTTP request itself failed (DNS, TLS, timeout, status code, …).
    #[error("update request failed: {0}")]
    Network(String),
    /// The response could not be understood (bad JSON, missing field, …).
    #[error("unexpected update response: {0}")]
    Response(String),
    /// A filesystem or process operation failed.
    #[error("update failed: {0}")]
    Io(String),
    /// [`CHECKSUMS_FILE_NAME`] has no entry for the downloaded file.
    #[error("no checksum entry for {0}")]
    MissingChecksum(String),
    /// Signing is configured but the release carries no signature file.
    #[error("release is missing {0}")]
    MissingSignature(String),
    /// The downloaded bytes do not match [`CHECKSUMS_FILE_NAME`].
    ///
    /// The file is deleted before this error is reported, and nothing is
    /// ever executed from it.
    #[error("checksum mismatch for {0}; the file was deleted")]
    HashMismatch(String),
}

/// One downloadable file attached to a GitHub release.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct AssetInfo {
    /// File name as published on the release, e.g. `mikrotik-rif_0.2.0_amd64.deb`.
    pub name: String,
    /// Direct download URL (`browser_download_url` in the API).
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

/// The subset of a GitHub release the updater cares about.
///
/// Field renames match the `releases/latest` JSON shape. Nullable strings are
/// `Option` because the API emits `null` for empty release names and bodies.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct ReleaseInfo {
    /// Release tag, e.g. `v0.2.0`.
    #[serde(rename = "tag_name")]
    pub tag: String,
    /// Human-readable release title, if the maintainer set one.
    #[serde(default)]
    pub name: Option<String>,
    /// Markdown release notes, if any.
    #[serde(default)]
    pub body: Option<String>,
    /// Web page of the release (the browser fallback target).
    #[serde(rename = "html_url")]
    pub page_url: String,
    /// Files attached to the release.
    #[serde(default)]
    pub assets: Vec<AssetInfo>,
}

impl ReleaseInfo {
    /// Release notes as plain text (empty when the release has none).
    #[must_use]
    pub fn notes(&self) -> &str {
        self.body.as_deref().unwrap_or("")
    }
}

/// Outcome of a background latest-release check.
#[derive(Debug)]
pub enum CheckOutcome {
    /// The release tag is newer than the running app.
    Available {
        /// The newer release.
        release: Box<ReleaseInfo>,
        /// ETag to send back as `If-None-Match` on the next check.
        etag: Option<String>,
    },
    /// The running app is up to date (or the tag could not be compared).
    UpToDate {
        /// ETag to send back as `If-None-Match` on the next check.
        etag: Option<String>,
    },
    /// The endpoint answered `304 Not Modified`: nothing changed since the last
    /// check, so whatever the previous outcome was still holds.
    NotModified,
    /// The check failed; the string is already user-readable.
    Failed(String),
}

/// Outcome events of a background installer download.
#[derive(Debug)]
pub enum DownloadOutcome {
    /// More bytes arrived: total bytes written so far and the advertised
    /// total, when the server sent a `Content-Length`.
    Progress {
        /// Bytes written to the destination file so far.
        done: u64,
        /// Advertised total, when known.
        total: Option<u64>,
    },
    /// The file was downloaded and its SHA-256 verified. Carries the expected
    /// digest so the handoff can re-verify the bytes immediately before use.
    Done {
        /// Verified file on disk.
        path: PathBuf,
        /// SHA-256 hex digest the file matched when it was downloaded.
        expected_hex: String,
    },
    /// The download or the verification failed; the string is user-readable.
    /// A partially written file is always removed first.
    Failed(String),
}

/// What [`perform_handoff`] did with the verified installer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoffAction {
    /// Windows: the installer was launched; the caller should exit.
    InstallerLaunched,
    /// Linux AppImage: the running image was replaced and relaunched; the
    /// caller should exit.
    #[allow(
        dead_code,
        reason = "constructed only by the Linux-only self-replace path; the variant stays on every target so the call sites match exhaustively"
    )]
    AppImageReplaced,
    /// macOS/Linux: the file was opened with the desktop handler.
    PackageOpened,
}

/// `User-Agent` sent to the GitHub API (required by the API; identifies the app).
#[must_use]
pub fn user_agent() -> String {
    format!("mikrotik-rif/{}", env!("CARGO_PKG_VERSION"))
}

/// URL of the `releases/latest` endpoint for this app's repository.
#[must_use]
pub fn latest_release_url() -> String {
    format!("https://api.github.com/repos/{UPDATE_OWNER}/{UPDATE_REPO}/releases/latest")
}

/// Web URL of the project's repository (About screen).
#[must_use]
pub fn repository_url() -> String {
    format!("https://github.com/{UPDATE_OWNER}/{UPDATE_REPO}")
}

/// Web URL of the releases page (browser fallback when no asset matches).
#[must_use]
pub fn releases_page_url() -> String {
    format!("{}/releases", repository_url())
}

/// Web URL of the licence file in the repository (About screen).
#[must_use]
pub fn license_url() -> String {
    format!("{}/blob/main/LICENSE", repository_url())
}

/// Validate an update URL before any request is made to it.
///
/// The release JSON is remote input: without this check a compromised API
/// response could hand the downloader a `file://`, custom-scheme, or
/// non-GitHub URL. Only `https://` on an [`ALLOWED_HOSTS`] host passes.
///
/// # Errors
///
/// Returns [`UpdateError::InvalidUrl`] for any other scheme or host.
pub fn check_url(url: &str) -> Result<(), UpdateError> {
    let invalid = || UpdateError::InvalidUrl(url.to_owned());
    let rest = url.strip_prefix("https://").ok_or_else(invalid)?;
    // Authority is everything before the first path/query/fragment delimiter.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    // Drop any `user:password@` prefix, then any `:port` suffix.
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .split(':')
        .next()
        .unwrap_or_default();
    if ALLOWED_HOSTS.contains(&host) {
        Ok(())
    } else {
        Err(invalid())
    }
}

/// Keep only a safe `ETag` value from an untrusted source.
///
/// The value travels in an HTTP header (`ETag` out, `If-None-Match` back) and
/// is also persisted through eframe storage, so it crosses two trust
/// boundaries: network input and stored input that a local process or a
/// corrupted profile could have altered. Anything longer than
/// [`MAX_ETAG_BYTES`] or containing bytes outside printable ASCII
/// (`0x20..=0x7E`, which already excludes `\r`, `\n` and every other control
/// or non-ASCII byte) is refused with `None`; otherwise the trimmed value is
/// returned.
///
/// Handoff note: the storage read itself lives outside this module
/// (`restore_update_prefs` in `src/app/update_ui.rs` loads `update.etag`, and
/// `App::save` in `src/app.rs` writes it back), which this task must not
/// touch. The boundary is therefore enforced where the stored value re-enters
/// this module — [`net::spawn_check`] and `fetch_latest_with` sanitize the
/// incoming conditional with this function — and where a network value leaves
/// it for storage (`fetch_latest_with` sanitizes the response `ETag` before
/// it is returned in `CheckOutcome`). Callers must never persist or echo an
/// unsanitized `ETag`.
#[must_use]
pub(crate) fn sanitize_etag(raw: &str) -> Option<String> {
    if raw.len() > MAX_ETAG_BYTES {
        return None;
    }
    if !raw.bytes().all(|byte| (0x20..=0x7E).contains(&byte)) {
        return None;
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Reject a release whose JSON payload is implausibly large.
///
/// The endpoint is GitHub over TLS, but an oversized body (many assets or very
/// long strings) would still burn memory and CPU on every check, and the
/// download path would then be handed an attacker-chosen asset list.
fn validate_release(release: &ReleaseInfo) -> Result<(), UpdateError> {
    if release.assets.len() > MAX_ASSETS {
        return Err(UpdateError::Response(format!(
            "release lists {} assets (limit {MAX_ASSETS})",
            release.assets.len()
        )));
    }
    if release.tag.len() > MAX_FIELD_BYTES || release.page_url.len() > MAX_FIELD_BYTES {
        return Err(UpdateError::Response("release field too long".to_owned()));
    }
    if release
        .name
        .as_deref()
        .is_some_and(|name| name.len() > MAX_FIELD_BYTES)
    {
        return Err(UpdateError::Response("release name too long".to_owned()));
    }
    if release
        .body
        .as_deref()
        .is_some_and(|body| body.len() > MAX_NOTES_BYTES)
    {
        return Err(UpdateError::Response("release notes too long".to_owned()));
    }
    if release
        .assets
        .iter()
        .any(|asset| asset.name.len() > MAX_FIELD_BYTES || asset.url.len() > MAX_FIELD_BYTES)
    {
        return Err(UpdateError::Response("asset field too long".to_owned()));
    }
    Ok(())
}

/// Current Unix time in seconds; `0` when the system clock is unavailable.
#[must_use]
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Whether an automatic check may run now.
///
/// The check runs once per launch while the user keeps it enabled: there is no
/// time-based budget, so starting the app always asks GitHub for the latest
/// release. Pure so the rule is unit-testable.
#[must_use]
pub const fn should_auto_check(enabled: bool) -> bool {
    enabled
}

/// Parse a release tag of the form `vX.Y.Z` (the `v` prefix is optional).
///
/// Returns `None` for anything that is not strict semantic versioning.
#[must_use]
pub fn parse_tag(tag: &str) -> Option<semver::Version> {
    let bare = tag
        .strip_prefix('v')
        .or_else(|| tag.strip_prefix('V'))
        .unwrap_or(tag);
    semver::Version::parse(bare).ok()
}

/// Whether `tag` is a newer stable release than `current`.
///
/// Pre-release tags never count as updates (see the module documentation).
/// Unparseable input conservatively reports "no update".
#[must_use]
pub fn is_update(current: &str, tag: &str) -> bool {
    let Ok(current_version) = semver::Version::parse(current) else {
        return false;
    };
    let Some(tag_version) = parse_tag(tag) else {
        return false;
    };
    tag_version.pre.is_empty() && tag_version > current_version
}

/// Architecture token used in the release asset names.
///
/// Maps a `std::env::consts::ARCH` value to the `amd64` / `arm64` spelling
/// that every artifact shares. `None` for architectures the project does not
/// ship, so the caller falls back to the release page.
#[must_use]
pub fn asset_arch(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("amd64"),
        "aarch64" => Some("arm64"),
        _ => None,
    }
}

/// Pick the release asset that installs this app on `(os, arch)`.
///
/// `os` and `arch` use the `std::env::consts` vocabulary (`"windows"`,
/// `"macos"`, `"linux"`; `"x86_64"`, `"aarch64"`). The matching rules are the
/// asset-name contract documented at the top of this module: anything not in
/// that table yields `None`, and the caller must fall back to the release
/// page instead of guessing.
///
/// This is deliberately pure (no environment reads): [`select_asset_for_current`]
/// adds the running-process preference on top.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
#[must_use]
pub fn select_asset<'a>(assets: &'a [AssetInfo], os: &str, arch: &str) -> Option<&'a AssetInfo> {
    let token = asset_arch(arch)?;
    let marker = format!("_{token}");
    match (os, token) {
        ("windows", _) => assets.iter().find(|asset| is_setup(asset, &marker)),
        // Only Apple Silicon is shipped; requiring the `_arm64` marker keeps an
        // accidental extra `.dmg` (say an Intel one) from being picked by order.
        ("macos", "arm64") => assets.iter().find(|asset| {
            is_release_asset(asset) && asset.name.ends_with(&format!("{marker}.dmg"))
        }),
        ("linux", _) => select_linux(assets, &marker),
        _ => None,
    }
}

/// Pick the release asset for the process that is actually running.
///
/// Identical to [`select_asset`] with the current `std::env::consts`, except
/// that a Linux process running from an AppImage prefers the `.AppImage`
/// asset so the update can replace it in place (see [`maybe_self_replace`]).
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
#[must_use]
pub fn select_asset_for_current(assets: &[AssetInfo]) -> Option<&AssetInfo> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    if os == "linux"
        && running_appimage().is_some()
        && let Some(token) = asset_arch(arch)
        && let Some(image) = find_appimage(assets, &format!("_{token}"))
    {
        return Some(image);
    }
    select_asset(assets, os, arch)
}

/// A Windows NSIS installer for the given architecture marker.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
fn is_setup(asset: &AssetInfo, arch_marker: &str) -> bool {
    is_release_asset(asset)
        && asset.name.ends_with("-setup.exe")
        && asset.name.contains(arch_marker)
}

/// Whether an asset name is one of this project's published release artifacts.
///
/// Anchoring on the shared prefix keeps unrelated attachments (checksums,
/// signatures, notes) from ever matching an installer rule.
fn is_release_asset(asset: &AssetInfo) -> bool {
    asset.name.starts_with(ASSET_PREFIX)
}

/// Linux preference order: native `.deb`, then `.rpm`, then `.AppImage`.
///
/// `marker` is the `_amd64` / `_arm64` token shared by every Linux artifact.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
fn select_linux<'a>(assets: &'a [AssetInfo], marker: &str) -> Option<&'a AssetInfo> {
    assets
        .iter()
        .find(|asset| {
            is_release_asset(asset) && asset.name.ends_with(".deb") && asset.name.contains(marker)
        })
        .or_else(|| {
            assets.iter().find(|asset| {
                is_release_asset(asset)
                    && asset.name.ends_with(".rpm")
                    && asset.name.contains(marker)
            })
        })
        .or_else(|| find_appimage(assets, marker))
}

/// The `.AppImage` asset for `marker` (`_amd64` / `_arm64`), shared by
/// [`select_linux`] and [`select_asset_for_current`].
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
fn find_appimage<'a>(assets: &'a [AssetInfo], marker: &str) -> Option<&'a AssetInfo> {
    assets.iter().find(|asset| {
        is_release_asset(asset) && asset.name.ends_with(".AppImage") && asset.name.contains(marker)
    })
}

/// Download URL of the release's [`CHECKSUMS_FILE_NAME`], if attached.
#[must_use]
pub fn checksums_url(release: &ReleaseInfo) -> Option<String> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS_FILE_NAME)
        .map(|asset| asset.url.clone())
}

/// Download URL of the release's detached minisign signature, if attached.
#[must_use]
pub fn signature_url(release: &ReleaseInfo) -> Option<String> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == SIGNATURE_FILE_NAME)
        .map(|asset| asset.url.clone())
}

/// Verify a minisign signature over `data` with a base64 public key.
///
/// # Errors
///
/// Returns [`UpdateError::Response`] when the key or the signature cannot be
/// parsed, or when the signature does not match.
pub fn verify_minisign(
    data: &[u8],
    signature_text: &str,
    public_key_base64: &str,
) -> Result<(), UpdateError> {
    // Accept either the bare base64 key (the `.pub` file's second line) or the
    // whole `.pub` file, so pasting the file into the repository variable
    // works too.
    let key_text = public_key_base64.trim();
    let key = minisign_verify::PublicKey::from_base64(key_text)
        .or_else(|_| minisign_verify::PublicKey::decode(key_text))
        .map_err(|error| UpdateError::Response(format!("invalid signing key: {error}")))?;
    let signature = minisign_verify::Signature::decode(signature_text)
        .map_err(|error| UpdateError::Response(format!("invalid signature: {error}")))?;
    key.verify(data, &signature, false)
        .map_err(|error| UpdateError::Response(format!("signature check failed: {error}")))
}

/// Extract the expected SHA-256 hex digest for `asset_name` from a
/// `SHA256SUMS.txt` body (`"<hex>  <file>"` lines, `*` binary marker tolerated).
///
/// Returns `None` when the file has no well-formed entry for the asset.
#[must_use]
pub fn find_checksum(sums: &str, asset_name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        if name == asset_name && is_hex_digest(hash) {
            Some(hash.to_ascii_lowercase())
        } else {
            None
        }
    })
}

/// Whether `hash` looks like a SHA-256 hex digest (64 hex characters).
#[must_use]
pub fn is_hex_digest(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Whether two hex digests match (case-insensitive).
///
/// Both sides are public checksums, never secrets, so a plain comparison is
/// the honest tool here.
fn digests_match(actual_hex: &str, expected_hex: &str) -> bool {
    actual_hex.eq_ignore_ascii_case(expected_hex)
}

/// SHA-256 hex digest of the file at `path`, streamed so large installers do
/// not have to fit in memory.
///
/// # Errors
///
/// Returns the underlying I/O error when the file cannot be read.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0_u8; CHUNK_LEN].into_boxed_slice();
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hasher.update(&chunk[..read]);
    }
    Ok(hex_encode(hasher.finalize().as_slice()))
}

/// Lowercase hexadecimal rendering of a byte slice.
///
/// Spelled out instead of `format!("{:x}")`: the digest crates return an
/// `Array` byte container that stopped implementing `LowerHex`.
fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// Collapse release notes to a single short line for the update banner.
///
/// Whitespace runs become single spaces; longer texts are cut at
/// [`MAX_NOTES_CHARS`] characters with an ellipsis.
#[must_use]
pub fn summarize_notes(notes: &str) -> String {
    let flat = notes.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= MAX_NOTES_CHARS {
        flat
    } else {
        let cut: String = flat.chars().take(MAX_NOTES_CHARS).collect();
        format!("{cut}…")
    }
}

/// One HTTP response, reduced to the fields the updater needs.
/// Compare the file at `dest` against `sums`; delete it on any mismatch.
fn verify_against_sums(sums: &str, asset_name: &str, dest: &Path) -> DownloadOutcome {
    let Some(expected) = find_checksum(sums, asset_name) else {
        let _ = std::fs::remove_file(dest);
        return DownloadOutcome::Failed(
            UpdateError::MissingChecksum(asset_name.to_owned()).to_string(),
        );
    };
    match hash_file(dest) {
        Ok(actual) if digests_match(&actual, &expected) => DownloadOutcome::Done {
            path: dest.to_owned(),
            expected_hex: expected,
        },
        Ok(_) => {
            let _ = std::fs::remove_file(dest);
            DownloadOutcome::Failed(UpdateError::HashMismatch(asset_name.to_owned()).to_string())
        }
        Err(error) => {
            let _ = std::fs::remove_file(dest);
            DownloadOutcome::Failed(UpdateError::Io(error.to_string()).to_string())
        }
    }
}

mod handoff;
mod net;
#[cfg(test)]
mod tests;

#[cfg(target_os = "linux")]
pub use handoff::maybe_self_replace;
pub use handoff::{open_release_page, perform_handoff, running_appimage};
pub use net::{asset_dest, spawn_check, spawn_download};
