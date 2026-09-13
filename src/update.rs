//! In-app updater: the only code in the product that opens a socket.
//!
//! Everything else in this product runs locally and never opens a socket. This
//! module owns the only exception: a poll of the GitHub Releases API plus the
//! download of a release installer. The rules are:
//!
//! * Automatic checks run at most once a day, on a background [`std::thread`]
//!   spawned at startup, and never block the interface thread.
//! * The user can disable automatic checks and can always trigger a manual one.
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

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Only the Linux self-replace path touches Unix permission bits; gating this
// to `unix` would leave it unused (and CI's `-D warnings` would fail) on
// macOS, where that path is not compiled.
#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt as _;

use sha2::{Digest as _, Sha256};

/// GitHub account that owns the releases polled by the updater.
///
/// Centralized here; see the module documentation for why.
pub const UPDATE_OWNER: &str = "balakar94";
/// Repository whose releases are polled by the updater.
///
/// Centralized here; see the module documentation for why.
pub const UPDATE_REPO: &str = "mikrotik-rif";
/// Minimum seconds between two automatic update checks (one day).
pub const AUTO_CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
/// Checksum file attached to every release by the release workflow.
pub const CHECKSUMS_FILE_NAME: &str = "SHA256SUMS.txt";
/// Longest release-notes excerpt shown in the update banner, in characters.
pub const MAX_NOTES_CHARS: usize = 280;
/// Network timeout for the small release-metadata request.
const METADATA_TIMEOUT: Duration = Duration::from_secs(25);
/// Network timeout for installer downloads (large files on slow links).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Streaming buffer used while writing a download to disk.
const CHUNK_LEN: usize = 64 * 1024;
/// Hosts allowed for update traffic. Release assets redirect to
/// `objects.githubusercontent.com`; every URL (initial and per redirect hop)
/// must land on one of these, so a compromised API response cannot point the
/// downloader at an arbitrary host.
const ALLOWED_HOSTS: [&str; 3] = [
    "github.com",
    "api.github.com",
    "objects.githubusercontent.com",
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
    Available(Box<ReleaseInfo>),
    /// The running app is up to date (or the tag could not be compared).
    UpToDate,
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

/// Whether an automatic check may run now (at most once a day).
///
/// Pure so the scheduling rule is unit-testable: the "once a day" budget must
/// hold even if the process restarts often.
#[must_use]
pub const fn should_auto_check(
    enabled: bool,
    last_check_unix: Option<u64>,
    now_unix_secs: u64,
) -> bool {
    if !enabled {
        return false;
    }
    match last_check_unix {
        None => true,
        Some(last) => now_unix_secs.saturating_sub(last) >= AUTO_CHECK_INTERVAL_SECS,
    }
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
        ("macos", "arm64") => assets.iter().find(|asset| asset.name.ends_with(".dmg")),
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
    if os == "linux" && running_appimage().is_some() {
        if let Some(token) = asset_arch(arch) {
            if let Some(image) = find_appimage(assets, &format!("_{token}")) {
                return Some(image);
            }
        }
    }
    select_asset(assets, os, arch)
}

/// A Windows NSIS installer for the given architecture marker.
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
fn is_setup(asset: &AssetInfo, arch_marker: &str) -> bool {
    asset.name.ends_with("-setup.exe") && asset.name.contains(arch_marker)
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
        .find(|asset| asset.name.ends_with(".deb") && asset.name.contains(marker))
        .or_else(|| {
            assets
                .iter()
                .find(|asset| asset.name.ends_with(".rpm") && asset.name.contains(marker))
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
    assets
        .iter()
        .find(|asset| asset.name.ends_with(".AppImage") && asset.name.contains(marker))
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

/// Query the `releases/latest` endpoint (blocking; call off the UI thread).
///
/// # Errors
///
/// Returns [`UpdateError::Network`] when the request fails and
/// [`UpdateError::Response`] when the body is not a recognizable release.
pub fn fetch_latest() -> Result<ReleaseInfo, UpdateError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(METADATA_TIMEOUT))
        .max_redirects(0)
        .build()
        .into();
    let mut response = get_following_redirects(&agent, &latest_release_url())?;
    let release: ReleaseInfo = response
        .body_mut()
        .with_config()
        .limit(MAX_METADATA_BYTES)
        .read_json()
        .map_err(|error| UpdateError::Response(error.to_string()))?;
    validate_release(&release)?;
    Ok(release)
}

/// Perform a `GET`, following at most [`MAX_REDIRECTS`] redirects manually so
/// that every hop is validated by [`check_url`].
///
/// `ureq`'s automatic following is turned off (`.max_redirects(0)`): it would
/// follow a `Location` header to any host, which is precisely what the
/// validation must prevent. GitHub release assets legitimately redirect to
/// `objects.githubusercontent.com`, so a fixed number of hops is still
/// permitted — just never off the allow-list.
fn get_following_redirects(
    agent: &ureq::Agent,
    start_url: &str,
) -> Result<ureq::http::Response<ureq::Body>, UpdateError> {
    let mut url = start_url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        check_url(&url)?;
        let response = agent
            .get(url.clone())
            .header("User-Agent", user_agent())
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        if !response.status().is_redirection() {
            return Ok(response);
        }
        url = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
            .ok_or_else(|| UpdateError::InvalidUrl(url.clone()))?;
    }
    Err(UpdateError::InvalidUrl(format!(
        "too many redirects from {start_url}"
    )))
}

/// Check for updates on a background thread; the single message arrives on
/// the returned channel. Never blocks the caller.
#[must_use]
pub fn spawn_check(current_version: String) -> Receiver<CheckOutcome> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = match fetch_latest() {
            Ok(release) if is_update(&current_version, &release.tag) => {
                CheckOutcome::Available(Box::new(release))
            }
            Ok(_) => CheckOutcome::UpToDate,
            Err(error) => CheckOutcome::Failed(error.to_string()),
        };
        let _ = sender.send(outcome);
    });
    receiver
}

/// Directory downloads are saved to: `~/Downloads` when it exists, otherwise
/// the system temporary directory. Windows always uses the temporary
/// directory because the installer is a throwaway launcher, not a keepsake.
#[must_use]
pub fn download_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        return std::env::temp_dir();
    }
    if let Some(home) = std::env::var_os("HOME") {
        let downloads = PathBuf::from(home).join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
    }
    std::env::temp_dir()
}

/// Destination path for an asset: [`download_dir`] plus the asset's file
/// name. Any directory components in a hostile asset name are stripped.
#[must_use]
pub fn asset_dest(asset_name: &str) -> PathBuf {
    let file_name = Path::new(asset_name)
        .file_name()
        .map_or("mikrotik-rif-update.bin", |name| {
            name.to_str().unwrap_or("mikrotik-rif-update.bin")
        });
    download_dir().join(file_name)
}

/// Download `asset` with progress events, then verify it against the
/// checksum file at `checksums_url`, all on a background thread.
///
/// The file is streamed straight to `dest` (never fully buffered in memory).
/// When verification fails — or the checksum entry is missing — the file is
/// deleted and [`DownloadOutcome::Failed`] is sent; [`DownloadOutcome::Done`]
/// is only sent for a verified file.
///
/// An empty `checksums_url` is treated as a missing checksum entry: without
/// a checksum there is nothing to verify against, so nothing is kept.
#[must_use]
pub fn spawn_download(
    asset: AssetInfo,
    checksums_url: String,
    dest: PathBuf,
) -> Receiver<DownloadOutcome> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let outcome = download_verified(&asset, &checksums_url, &dest, &|done, total| {
            let _ = sender.send(DownloadOutcome::Progress { done, total });
        });
        let _ = sender.send(outcome);
    });
    receiver
}

/// Blocking download plus verification; see [`spawn_download`].
fn download_verified(
    asset: &AssetInfo,
    checksums_url: &str,
    dest: &Path,
    progress: &dyn Fn(u64, Option<u64>),
) -> DownloadOutcome {
    if let Err(error) = download_file(&asset.url, dest, progress) {
        let _ = std::fs::remove_file(dest);
        return DownloadOutcome::Failed(error.to_string());
    }
    let sums = match fetch_text(checksums_url) {
        Ok(sums) => sums,
        Err(error) => {
            let _ = std::fs::remove_file(dest);
            return DownloadOutcome::Failed(error.to_string());
        }
    };
    verify_against_sums(&sums, &asset.name, dest)
}

/// Stream `url` to `dest`, reporting `(bytes_written, advertised_total)`.
///
/// The body is bounded by [`MAX_INSTALLER_BYTES`], both from the advertised
/// `Content-Length` and while streaming, so a hostile or broken endpoint
/// cannot fill the disk.
fn download_file(
    url: &str,
    dest: &Path,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<(), UpdateError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .max_redirects(0)
        .build()
        .into();
    let mut response = get_following_redirects(&agent, url)?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if total.is_some_and(|total| total > MAX_INSTALLER_BYTES) {
        return Err(too_large());
    }
    let mut reader = response.body_mut().as_reader();
    let mut file =
        std::fs::File::create(dest).map_err(|error| UpdateError::Io(error.to_string()))?;
    let mut chunk = vec![0_u8; CHUNK_LEN].into_boxed_slice();
    let mut done: u64 = 0;
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        if read == 0 {
            break;
        }
        done += u64::try_from(read).unwrap_or(u64::MAX);
        if done > MAX_INSTALLER_BYTES {
            return Err(too_large());
        }
        std::io::Write::write_all(&mut file, &chunk[..read])
            .map_err(|error| UpdateError::Io(error.to_string()))?;
        progress(done, total);
    }
    Ok(())
}

/// Error for a download that exceeds [`MAX_INSTALLER_BYTES`].
fn too_large() -> UpdateError {
    UpdateError::Response(format!(
        "installer larger than {} MiB",
        MAX_INSTALLER_BYTES / (1024 * 1024)
    ))
}

/// Fetch a small text body (used for [`CHECKSUMS_FILE_NAME`]), bounded by
/// [`MAX_CHECKSUMS_BYTES`] and validated like every other update URL.
fn fetch_text(url: &str) -> Result<String, UpdateError> {
    if url.is_empty() {
        return Err(UpdateError::MissingChecksum(CHECKSUMS_FILE_NAME.to_owned()));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(METADATA_TIMEOUT))
        .max_redirects(0)
        .build()
        .into();
    let mut response = get_following_redirects(&agent, url)?;
    response
        .body_mut()
        .with_config()
        .limit(MAX_CHECKSUMS_BYTES)
        .read_to_string()
        .map_err(|error| UpdateError::Response(error.to_string()))
}

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

/// Re-hash `path` and compare it with the digest captured at download time.
///
/// The file exists between verification and use, and a local process with
/// write access to the download directory could swap it in that window. The
/// file is deleted on a mismatch so nothing unverified can be launched.
fn reverify(path: &Path, expected_hex: &str) -> Result<(), UpdateError> {
    match hash_file(path) {
        Ok(actual) if digests_match(&actual, expected_hex) => Ok(()),
        Ok(_) => {
            let _ = std::fs::remove_file(path);
            Err(UpdateError::HashMismatch(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("update")
                    .to_owned(),
            ))
        }
        Err(error) => Err(UpdateError::Io(error.to_string())),
    }
}

/// Path of the AppImage this process runs from, if any.
///
/// The AppImage runtime exports `$APPIMAGE` with the image's own path; that
/// variable is the documented way to locate the running file. Only meaningful
/// on Linux, so anything else reports `None` without reading the environment.
#[must_use]
pub fn running_appimage() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    running_appimage_from(std::env::var_os("APPIMAGE"))
}

/// [`running_appimage`] over an explicit value, so the rule is unit-testable
/// without touching the process environment.
#[must_use]
pub fn running_appimage_from(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let path = PathBuf::from(value?);
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

/// Replace the running AppImage with a verified download and relaunch it.
///
/// `expected_hex` is the digest the download was verified against; it is
/// checked again here, immediately before the copy, so a file swapped on disk
/// after the download is refused. Replacement is a copy to a `<name>.update`
/// sibling plus an atomic rename in the same directory, so a crash can never
/// leave a half-written executable behind; the staging file is removed on any
/// error. The current process keeps running the old bytes — the caller should
/// exit right after, which the relaunched child (spawned with the same
/// arguments) makes seamless.
///
/// # Errors
///
/// Returns [`UpdateError::Io`] when the copy, rename, permission change, or
/// relaunch fails, and [`UpdateError::HashMismatch`] if the verified file no
/// longer matches. The running image is left untouched unless the final rename
/// succeeded.
#[cfg(target_os = "linux")]
pub fn replace_running_appimage(
    verified_new: &Path,
    expected_hex: &str,
) -> Result<HandoffAction, UpdateError> {
    let Some(current) = running_appimage() else {
        return Err(UpdateError::Io("not running from an AppImage".to_owned()));
    };
    // `$APPIMAGE` comes from the process environment, so it is treated as
    // untrusted input: only a regular file may become the replacement target.
    check_appimage_target(&current)?;
    if same_file(&current, verified_new) {
        relaunch(&current)?;
        return Ok(HandoffAction::AppImageReplaced);
    }
    reverify(verified_new, expected_hex)?;
    install_appimage_file(&current, verified_new)?;
    relaunch(&current)?;
    Ok(HandoffAction::AppImageReplaced)
}

/// Whether `verified` should replace the running AppImage instead of going
/// through the desktop handler.
///
/// Returns `None` when self-replace does not apply (not Linux, not running
/// from an AppImage, or the verified file is not an `.AppImage`); the caller
/// then falls back to [`perform_handoff`]. A `Some` value always ends the
/// story: success means the replacement was relaunched and the caller should
/// exit, failure means the running image is untouched and the error should be
/// shown.
#[cfg(target_os = "linux")]
#[allow(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "asset names are byte-exact CI artifacts; case-insensitive matching would widen the contract"
)]
pub fn maybe_self_replace(
    verified: &Path,
    expected_hex: &str,
) -> Option<Result<HandoffAction, UpdateError>> {
    running_appimage()?;
    let name = verified.file_name()?.to_str()?;
    if !name.ends_with(".AppImage") {
        return None;
    }
    Some(replace_running_appimage(verified, expected_hex))
}

/// Install `new_file` over `current` via a staging sibling plus atomic rename.
///
/// Both paths must be distinct (see [`same_file`]); the staging file lives
/// next to `current` so the rename never crosses filesystems.
#[cfg(target_os = "linux")]
fn install_appimage_file(current: &Path, new_file: &Path) -> Result<(), UpdateError> {
    let staging_name = match current.file_name().and_then(|name| name.to_str()) {
        Some(name) => format!("{name}.update"),
        None => {
            return Err(UpdateError::Io(
                "cannot stage beside the AppImage".to_owned(),
            ));
        }
    };
    let staging = current.with_file_name(staging_name);
    let failed = |reason: String| {
        let _ = std::fs::remove_file(&staging);
        UpdateError::Io(reason)
    };
    if let Err(error) = std::fs::copy(new_file, &staging) {
        return Err(failed(error.to_string()));
    }
    #[cfg(unix)]
    if let Err(error) = std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)) {
        return Err(failed(error.to_string()));
    }
    if let Err(error) = std::fs::rename(&staging, current) {
        return Err(failed(error.to_string()));
    }
    Ok(())
}

/// Reject an `$APPIMAGE` value that is not a regular file.
///
/// The variable is attacker-influenced when the launcher's environment is, so
/// a directory, symlink chain to nowhere, or missing path must never become
/// the overwrite target. Pure over its argument, so it is unit-testable
/// without touching the process environment.
#[cfg(target_os = "linux")]
fn check_appimage_target(current: &Path) -> Result<(), UpdateError> {
    if current.is_file() {
        Ok(())
    } else {
        Err(UpdateError::Io(
            "the running AppImage is not a regular file".to_owned(),
        ))
    }
}

/// Whether both paths point at the same file (handles symlinks and relative
/// spellings); conservative — any I/O error reports "different".
#[cfg(target_os = "linux")]
fn same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Spawn `path` with this process's arguments (minus argv[0]) and return
/// immediately; the child re-execs through the AppImage runtime on its own.
#[cfg(target_os = "linux")]
fn relaunch(path: &Path) -> Result<(), UpdateError> {
    std::process::Command::new(path)
        .args(std::env::args_os().skip(1))
        .spawn()
        .map_err(|error| UpdateError::Io(error.to_string()))?;
    Ok(())
}

/// Hand a verified installer to the operating system (never runs it here
/// beyond what the OS handler itself does; see the module documentation).
///
/// * Windows launches the `-setup.exe` as a detached child process; the
///   caller should close the app afterwards so the installer can replace it.
/// * macOS and Linux open the file with the desktop handler (`open` /
///   `xdg-open` via the [`open`] crate) so the user finishes the install.
///
/// `expected_hex` is re-checked immediately before the launch/open, closing
/// the window between download verification and use.
///
/// # Errors
///
/// Returns [`UpdateError::Io`] when the launch or open request fails, and
/// [`UpdateError::HashMismatch`] if the file no longer matches its digest.
pub fn perform_handoff(path: &Path, expected_hex: &str) -> Result<HandoffAction, UpdateError> {
    reverify(path, expected_hex)?;
    if cfg!(target_os = "windows") {
        std::process::Command::new(path)
            .spawn()
            .map_err(|error| UpdateError::Io(error.to_string()))?;
        Ok(HandoffAction::InstallerLaunched)
    } else {
        open::that(path).map_err(|error| UpdateError::Io(error.to_string()))?;
        Ok(HandoffAction::PackageOpened)
    }
}

/// Open the release page in the browser (fallback when no asset matches the
/// current platform). The URL is validated first: it comes from the API JSON.
///
/// # Errors
///
/// Returns [`UpdateError::InvalidUrl`] for a non-GitHub URL and
/// [`UpdateError::Io`] when no browser could be launched.
pub fn open_release_page(page_url: &str) -> Result<(), UpdateError> {
    check_url(page_url)?;
    open::that(page_url).map_err(|error| UpdateError::Io(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> AssetInfo {
        AssetInfo {
            name: name.to_owned(),
            url: format!("https://example.com/{name}"),
        }
    }

    fn realistic_assets() -> Vec<AssetInfo> {
        vec![
            asset("mikrotik-rif_0.2.0_amd64-setup.exe"),
            asset("mikrotik-rif_0.2.0_arm64-setup.exe"),
            asset("mikrotik-rif_0.2.0_arm64.dmg"),
            asset("mikrotik-rif_0.2.0_amd64.deb"),
            asset("mikrotik-rif_0.2.0_arm64.deb"),
            asset("mikrotik-rif_0.2.0_amd64.rpm"),
            asset("mikrotik-rif_0.2.0_arm64.rpm"),
            asset("mikrotik-rif_0.2.0_amd64.AppImage"),
            asset("mikrotik-rif_0.2.0_arm64.AppImage"),
            asset("SHA256SUMS.txt"),
        ]
    }

    #[test]
    fn endpoint_uses_the_central_coordinates() {
        let url = latest_release_url();
        assert!(url.contains(UPDATE_OWNER), "owner missing in {url}");
        assert!(url.contains(UPDATE_REPO), "repo missing in {url}");
        assert!(
            url.ends_with("/releases/latest"),
            "unexpected endpoint {url}"
        );
    }

    #[test]
    fn user_agent_identifies_the_app() {
        let agent = user_agent();
        assert!(
            agent.starts_with("mikrotik-rif/"),
            "unexpected agent {agent}"
        );
        assert!(agent.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn tag_parsing_accepts_an_optional_v_prefix() {
        assert_eq!(
            parse_tag("v1.2.3"),
            Some(semver::Version::new(1, 2, 3)),
            "v prefix"
        );
        assert_eq!(
            parse_tag("1.2.3"),
            Some(semver::Version::new(1, 2, 3)),
            "bare version"
        );
        assert!(parse_tag("not-a-version").is_none());
        assert!(parse_tag("v1.2").is_none(), "partial versions are rejected");
        assert!(parse_tag("").is_none());
    }

    #[test]
    fn newer_stable_tags_are_updates() {
        assert!(is_update("0.1.0", "v0.2.0"));
        assert!(is_update("0.1.0", "1.0.0"));
        assert!(!is_update("0.2.0", "v0.2.0"), "equal is not newer");
        assert!(!is_update("0.3.0", "v0.2.0"), "older is not newer");
        assert!(!is_update("0.1.0", "garbage"));
        assert!(!is_update("garbage", "v9.9.9"), "bad current never updates");
    }

    #[test]
    fn pre_release_tags_are_ignored() {
        // `releases/latest` never points at a pre-release; this guards the
        // comparison itself so a hand-crafted tag cannot sneak one through.
        assert!(!is_update("0.1.0", "v9.9.9-beta.1"));
        assert!(!is_update("0.1.0", "v0.2.0-rc.1"));
        assert!(!is_update("1.2.2", "v1.2.3-alpha"));
    }

    #[test]
    fn running_version_parses() {
        assert!(
            semver::Version::parse(env!("CARGO_PKG_VERSION")).is_ok(),
            "CARGO_PKG_VERSION must be semantic versioning"
        );
    }

    #[test]
    fn windows_assets_match_by_arch_marker() {
        let assets = realistic_assets();
        assert_eq!(
            select_asset(&assets, "windows", "x86_64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_amd64-setup.exe")
        );
        assert_eq!(
            select_asset(&assets, "windows", "aarch64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_arm64-setup.exe")
        );
        assert!(
            select_asset(&assets, "windows", "x86").is_none(),
            "32-bit Windows is not shipped"
        );
    }

    #[test]
    fn macos_only_serves_apple_silicon() {
        let assets = realistic_assets();
        assert_eq!(
            select_asset(&assets, "macos", "aarch64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_arm64.dmg")
        );
        assert!(
            select_asset(&assets, "macos", "x86_64").is_none(),
            "Intel macOS is not shipped"
        );
    }

    #[test]
    fn linux_prefers_native_packages_over_appimage() {
        let assets = realistic_assets();
        assert_eq!(
            select_asset(&assets, "linux", "x86_64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_amd64.deb"),
            "deb wins on x86_64"
        );
        assert_eq!(
            select_asset(&assets, "linux", "aarch64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_arm64.deb"),
            "deb wins on aarch64"
        );
    }

    #[test]
    fn linux_falls_back_through_rpm_to_appimage() {
        let without_deb = vec![
            asset("mikrotik-rif_0.2.0_amd64.rpm"),
            asset("mikrotik-rif_0.2.0_amd64.AppImage"),
            asset("mikrotik-rif_0.2.0_arm64.rpm"),
            asset("mikrotik-rif_0.2.0_arm64.AppImage"),
        ];
        assert_eq!(
            select_asset(&without_deb, "linux", "x86_64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_amd64.rpm")
        );
        let portable_only = vec![
            asset("mikrotik-rif_0.2.0_amd64.AppImage"),
            asset("mikrotik-rif_0.2.0_arm64.AppImage"),
        ];
        assert_eq!(
            select_asset(&portable_only, "linux", "aarch64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_arm64.AppImage")
        );
    }

    #[test]
    fn linux_never_crosses_architectures() {
        let x64_only = vec![
            asset("mikrotik-rif_0.2.0_amd64.deb"),
            asset("mikrotik-rif_0.2.0_amd64.rpm"),
            asset("mikrotik-rif_0.2.0_amd64.AppImage"),
        ];
        assert!(
            select_asset(&x64_only, "linux", "aarch64").is_none(),
            "amd64 assets must not serve arm64"
        );
        assert!(
            select_asset(&[], "linux", "x86_64").is_none(),
            "empty release has no asset"
        );
    }

    #[test]
    fn unknown_platforms_fall_back_to_the_release_page() {
        let assets = realistic_assets();
        assert!(select_asset(&assets, "freebsd", "x86_64").is_none());
        assert!(select_asset(&assets, "windows", "riscv64").is_none());
    }

    #[test]
    fn checksums_url_finds_the_attached_sums() {
        let release = ReleaseInfo {
            tag: "v0.2.0".to_owned(),
            name: None,
            body: None,
            page_url: "https://example.com/release".to_owned(),
            assets: realistic_assets(),
        };
        let url = checksums_url(&release).expect("SHA256SUMS.txt is attached");
        assert!(url.ends_with("SHA256SUMS.txt"), "unexpected url {url}");
        let bare = ReleaseInfo {
            assets: vec![asset("mikrotik-rif_0.2.0_amd64.deb")],
            ..release
        };
        assert!(checksums_url(&bare).is_none());
    }

    #[test]
    fn checksum_lookup_parses_sums_lines() {
        let sums = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  mikrotik-rif_0.2.0_amd64.deb\n\
                    9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 *other.deb\n";
        assert_eq!(
            find_checksum(sums, "mikrotik-rif_0.2.0_amd64.deb"),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned())
        );
        assert_eq!(
            find_checksum(sums, "other.deb"),
            Some("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".to_owned()),
            "binary marker tolerated"
        );
        assert!(find_checksum(sums, "missing.deb").is_none());
        assert!(find_checksum("not a sums line\n", "a").is_none());
        assert!(
            find_checksum("xyz  a.deb\n", "a.deb").is_none(),
            "short hash rejected"
        );
    }

    #[test]
    fn sha256_known_vector_without_network() {
        // Well-known SHA-256 of "abc"; local data only.
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let digest = hex_encode(Sha256::digest(b"abc").as_slice());
        assert_eq!(digest, expected);
        assert!(digests_match(&digest, &expected.to_ascii_uppercase()));
        assert!(!digests_match(&digest, "00"));
        assert_eq!(hex_encode(&[]), "");
        assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
    }

    /// Scratch file unique to this test process and counter.
    fn scratch_file(contents: &[u8]) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "mikrotik-rif-update-test-{}-{id}.bin",
            std::process::id()
        ));
        std::fs::write(&path, contents).expect("scratch file is writable");
        path
    }

    #[test]
    fn verified_files_are_kept_and_bad_ones_deleted() {
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let sums = format!("{expected}  installer.bin\n");

        let kept = scratch_file(b"abc");
        match verify_against_sums(&sums, "installer.bin", &kept) {
            DownloadOutcome::Done { path, expected_hex } => {
                assert_eq!(path, kept);
                assert_eq!(expected_hex, expected);
            }
            other => panic!("verified file must be kept, got {other:?}"),
        }
        assert!(kept.is_file(), "verified file stays on disk");
        std::fs::remove_file(&kept).expect("cleanup");

        let tampered = scratch_file(b"abd");
        match verify_against_sums(&sums, "installer.bin", &tampered) {
            DownloadOutcome::Failed(_) => {}
            other => panic!("tampered file must fail, got {other:?}"),
        }
        assert!(
            !tampered.exists(),
            "tampered file is deleted, never executed"
        );

        let unknown = scratch_file(b"abc");
        match verify_against_sums(&sums, "other.bin", &unknown) {
            DownloadOutcome::Failed(_) => {}
            other => panic!("missing entry must fail, got {other:?}"),
        }
        assert!(!unknown.exists(), "unverifiable file is deleted");
    }

    #[test]
    fn notes_are_collapsed_and_capped() {
        assert_eq!(summarize_notes(""), "");
        assert_eq!(
            summarize_notes("  line one\nline   two  "),
            "line one line two"
        );
        let long = "w ".repeat(MAX_NOTES_CHARS);
        let summary = summarize_notes(&long);
        assert!(summary.ends_with('…'), "long notes are marked as cut");
        assert!(
            summary.chars().count() <= MAX_NOTES_CHARS + 1,
            "notes stay within budget"
        );
    }

    #[test]
    fn auto_check_runs_at_most_once_a_day() {
        assert!(should_auto_check(true, None, 1_000));
        assert!(should_auto_check(true, Some(0), AUTO_CHECK_INTERVAL_SECS));
        assert!(!should_auto_check(
            true,
            Some(100),
            100 + AUTO_CHECK_INTERVAL_SECS - 1
        ));
        assert!(!should_auto_check(false, None, u64::MAX), "opt-out wins");
        assert!(
            !should_auto_check(true, Some(9_000), 1_000),
            "clock skew never triggers"
        );
    }

    #[test]
    fn appimage_env_reports_the_running_image() {
        use std::ffi::OsString;
        assert_eq!(
            running_appimage_from(Some(OsString::from("/opt/app.AppImage"))),
            Some(PathBuf::from("/opt/app.AppImage"))
        );
        assert_eq!(running_appimage_from(None), None);
        assert_eq!(running_appimage_from(Some(OsString::new())), None);
    }

    #[test]
    fn appimage_lookup_finds_the_portable_asset() {
        let assets = realistic_assets();
        assert_eq!(
            find_appimage(&assets, "_amd64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_amd64.AppImage")
        );
        assert_eq!(
            find_appimage(&assets, "_arm64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_arm64.AppImage")
        );
        assert!(find_appimage(&assets, "riscv64").is_none());
        // The preference is not a filter: `select_asset` still prefers the
        // native package; only `select_asset_for_current` promotes the image
        // for a running AppImage.
        assert_eq!(
            select_asset(&assets, "linux", "x86_64").map(|found| found.name.as_str()),
            Some("mikrotik-rif_0.2.0_amd64.deb")
        );
    }

    /// Unique scratch directory per test (tests share one process, so the
    /// name combines the pid with an atomic counter).
    #[cfg(target_os = "linux")]
    fn scratch_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "mikrotik-rif-update-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn self_replace_ignores_non_appimage_files() {
        // No environment involved: a native package never self-replaces.
        assert!(maybe_self_replace(Path::new("mikrotik-rif_0.2.0_amd64.deb"), "00").is_none());
        assert!(maybe_self_replace(Path::new("release-notes.txt"), "00").is_none());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn appimage_target_must_be_a_regular_file() {
        let dir = scratch_dir();
        let file = dir.join("mikrotik-rif.AppImage");
        std::fs::write(&file, b"image").expect("write file");
        assert!(check_appimage_target(&file).is_ok());
        assert!(
            check_appimage_target(&dir).is_err(),
            "a directory is not a valid AppImage target"
        );
        assert!(
            check_appimage_target(&dir.join("missing.AppImage")).is_err(),
            "a missing path is not a valid AppImage target"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn appimage_install_replaces_atomically() {
        let dir = scratch_dir();
        let current = dir.join("mikrotik-rif.AppImage");
        let new_file = dir.join("downloaded.AppImage");
        std::fs::write(&current, b"old-bytes").expect("write current");
        std::fs::write(&new_file, b"new-bytes").expect("write new");

        install_appimage_file(&current, &new_file).expect("replace");
        assert_eq!(std::fs::read(&current).expect("read back"), b"new-bytes");
        assert!(
            !dir.join("mikrotik-rif.AppImage.update").exists(),
            "staging file is gone after the rename"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&current)
                .expect("metadata")
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "replaced image stays executable");
        }

        // A missing download never touches the running image.
        let missing = dir.join("absent.AppImage");
        assert!(install_appimage_file(&current, &missing).is_err());
        assert_eq!(std::fs::read(&current).expect("read back"), b"new-bytes");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_https_github_urls_are_allowed() {
        for url in [
            "https://github.com/balakar94/mikrotik-rif/releases/download/v1/x.deb",
            "https://api.github.com/repos/balakar94/mikrotik-rif/releases/latest",
            "https://objects.githubusercontent.com/github-production-release-asset/x",
        ] {
            assert!(check_url(url).is_ok(), "{url} must be accepted");
        }
        for url in [
            "http://github.com/x",
            "file:///etc/passwd",
            "evil://payload",
            "https://evil.example.com/x",
            "https://github.com.evil.example.com/x",
            "https://raw.githubusercontent.com/x",
            "not a url",
            "",
        ] {
            assert!(
                matches!(check_url(url), Err(UpdateError::InvalidUrl(_))),
                "{url} must be refused"
            );
        }
    }

    #[test]
    fn asset_dest_strips_path_traversal() {
        let traversal = asset_dest("../../evil.exe");
        assert_eq!(
            traversal.file_name().and_then(|name| name.to_str()),
            Some("evil.exe"),
            "directory components are stripped"
        );
        assert!(
            traversal.starts_with(download_dir()),
            "destination stays inside the download directory"
        );
        let empty = asset_dest("");
        assert_eq!(
            empty.file_name().and_then(|name| name.to_str()),
            Some("mikrotik-rif-update.bin"),
            "a nameless asset gets a safe fallback"
        );
    }

    #[test]
    fn oversized_release_payloads_are_rejected() {
        let mut release = ReleaseInfo {
            tag: "v0.2.0".to_owned(),
            name: None,
            body: None,
            page_url: "https://github.com/balakar94/mikrotik-rif/releases/tag/v0.2.0".to_owned(),
            assets: Vec::new(),
        };
        assert!(
            validate_release(&release).is_ok(),
            "a normal release passes"
        );

        release.assets = (0..=MAX_ASSETS)
            .map(|index| AssetInfo {
                name: format!("asset-{index}.bin"),
                url: format!("https://github.com/asset-{index}"),
            })
            .collect();
        assert!(
            matches!(validate_release(&release), Err(UpdateError::Response(_))),
            "too many assets is rejected"
        );

        release.assets = vec![asset("ok.bin")];
        release.body = Some("x".repeat(MAX_NOTES_BYTES + 1));
        assert!(
            matches!(validate_release(&release), Err(UpdateError::Response(_))),
            "oversized notes are rejected"
        );
        release.body = None;
        release.tag = "v".repeat(MAX_FIELD_BYTES + 1);
        assert!(
            matches!(validate_release(&release), Err(UpdateError::Response(_))),
            "oversized identifier fields are rejected"
        );
    }

    #[test]
    fn reverify_refuses_a_file_swapped_after_verification() {
        let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let file = scratch_file(b"abc");
        assert!(reverify(&file, good).is_ok(), "matching digest passes");

        // A local process swaps the verified file before it is used.
        std::fs::write(&file, b"tampered").expect("overwrite");
        assert!(
            matches!(reverify(&file, good), Err(UpdateError::HashMismatch(_))),
            "a swapped file is refused"
        );
        assert!(!file.exists(), "the swapped file is deleted");
    }
}
