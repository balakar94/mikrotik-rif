//! HTTP transport, release-metadata fetch and installer download.
//!
//! Split out of `update.rs`: every network call in the product goes through
//! this module, and the transport sits behind `HttpGet` so redirect handling
//! and downloads can be tested without a network.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use super::*;

///
/// The body is a reader, not a `Vec`, so a large installer streams to disk
/// instead of being buffered in memory.
pub struct HttpResponse {
    status: u16,
    location: Option<String>,
    content_length: Option<u64>,
    etag: Option<String>,
    body: Box<dyn Read>,
}

impl HttpResponse {
    /// Build a response. Shared by the real transport and by test doubles.
    #[must_use]
    pub fn new(
        status: u16,
        location: Option<String>,
        content_length: Option<u64>,
        etag: Option<String>,
        body: Box<dyn Read>,
    ) -> Self {
        Self {
            status,
            location,
            content_length,
            etag,
            body,
        }
    }

    /// HTTP status code.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// `Location` header, for redirects.
    #[must_use]
    pub fn location(&self) -> Option<&str> {
        self.location.as_deref()
    }

    /// Advertised body size, when the server sent `Content-Length`.
    #[must_use]
    pub fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// `ETag` header, when present.
    #[must_use]
    pub fn etag(&self) -> Option<&str> {
        self.etag.as_deref()
    }

    /// Consume the response and return its body reader.
    #[must_use]
    pub fn into_body(self) -> Box<dyn Read> {
        self.body
    }
}

/// Parameters of one `GET`, including the conditional-request header.
pub struct HttpRequest<'a> {
    /// Absolute URL to fetch.
    pub url: &'a str,
    /// `Accept` header value (empty to omit).
    pub accept: &'a str,
    /// `If-None-Match` value from a previous response, when known.
    pub if_none_match: Option<&'a str>,
}

/// Minimal HTTP surface the updater needs, injectable so redirect handling and
/// downloads can be tested without a network.
pub trait HttpGet {
    /// Perform one `GET` without following redirects (the updater follows and
    /// validates every hop itself).
    ///
    /// # Errors
    ///
    /// Returns [`UpdateError::Network`] when the request itself fails.
    fn get(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, UpdateError>;
}

/// Production transport over `ureq`.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    /// Transport with the given per-request timeout.
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .build()
            .into();
        Self { agent }
    }
}

impl HttpGet for UreqTransport {
    fn get(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, UpdateError> {
        let mut builder = self
            .agent
            .get(request.url)
            .header("User-Agent", user_agent());
        if !request.accept.is_empty() {
            builder = builder.header("Accept", request.accept);
        }
        if let Some(etag) = request.if_none_match {
            builder = builder.header("If-None-Match", etag);
        }
        let response = builder
            .call()
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        let status = response.status().as_u16();
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(ToOwned::to_owned)
        };
        let location = header("location");
        let etag = header("etag");
        let content_length = header("content-length").and_then(|value| value.parse::<u64>().ok());
        let body: Box<dyn Read> = Box::new(response.into_body().into_reader());
        Ok(HttpResponse::new(
            status,
            location,
            content_length,
            etag,
            body,
        ))
    }
}

/// Outcome of one release-metadata request.
pub enum FetchOutcome {
    /// The endpoint returned a release body.
    Modified {
        /// Parsed release payload.
        release: ReleaseInfo,
        /// ETag to echo back on the next check.
        etag: Option<String>,
    },
    /// The endpoint answered `304 Not Modified`.
    NotModified,
}

/// Query the `releases/latest` endpoint against an injected transport,
/// carrying the conditional request in and the new ETag out.
///
/// Blocking; [`spawn_check`] runs it off the UI thread.
pub(super) fn fetch_latest_with(
    transport: &dyn HttpGet,
    etag: Option<&str>,
) -> Result<FetchOutcome, UpdateError> {
    let response = get_following_redirects(transport, &latest_release_url(), ACCEPT_JSON, etag)?;
    if response.status() == 304 {
        return Ok(FetchOutcome::NotModified);
    }
    if !(200..300).contains(&response.status()) {
        return Err(UpdateError::Network(format!(
            "release request returned HTTP {}",
            response.status()
        )));
    }
    let etag = response.etag().map(ToOwned::to_owned);
    let body = read_limited(response.into_body(), MAX_METADATA_BYTES)?;
    let release: ReleaseInfo =
        serde_json::from_slice(&body).map_err(|error| UpdateError::Response(error.to_string()))?;
    validate_release(&release)?;
    Ok(FetchOutcome::Modified { release, etag })
}

/// Read a body up to `limit` bytes, refusing anything larger.
fn read_limited(reader: Box<dyn Read>, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut buffer = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut buffer)
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    if u64::try_from(buffer.len()).unwrap_or(u64::MAX) > limit {
        return Err(UpdateError::Response(format!(
            "response body larger than {limit} bytes"
        )));
    }
    Ok(buffer)
}

/// Perform a `GET`, following at most [`MAX_REDIRECTS`] redirects manually so
/// that every hop is validated by [`check_url`].
///
/// `ureq`'s automatic following is turned off (`.max_redirects(0)`): it would
/// follow a `Location` header to any host, which is precisely what the
/// validation must prevent. GitHub release assets legitimately redirect to an
/// object-store host, so a fixed number of hops is still permitted — just never
/// off the allow-list.
pub(super) fn get_following_redirects(
    transport: &dyn HttpGet,
    start_url: &str,
    accept: &str,
    if_none_match: Option<&str>,
) -> Result<HttpResponse, UpdateError> {
    let mut url = start_url.to_owned();
    let mut conditional = if_none_match.map(ToOwned::to_owned);
    for _ in 0..=MAX_REDIRECTS {
        check_url(&url)?;
        let response = transport.get(&HttpRequest {
            url: &url,
            accept,
            if_none_match: conditional.as_deref(),
        })?;
        // `304 Not Modified` is a terminal conditional response, not a redirect
        // to follow (it carries no `Location`).
        if response.status() == 304 || !(300..400).contains(&response.status()) {
            return Ok(response);
        }
        // A conditional request only makes sense on the original resource.
        conditional = None;
        let location = response
            .location()
            .ok_or_else(|| UpdateError::InvalidUrl(url.clone()))?;
        url = resolve_redirect(&url, location)?;
    }
    Err(UpdateError::InvalidUrl(format!(
        "too many redirects from {start_url}"
    )))
}

/// Resolve a `Location` header against the URL it came from.
///
/// Handles absolute and scheme-relative URLs plus path-relative locations. The
/// result is validated by [`check_url`] before it is requested, so this only
/// has to build a candidate.
pub(super) fn resolve_redirect(base: &str, location: &str) -> Result<String, UpdateError> {
    let location = location.trim();
    if location.is_empty() {
        return Err(UpdateError::InvalidUrl(base.to_owned()));
    }
    if location.starts_with("https://") || location.starts_with("http://") {
        return Ok(location.to_owned());
    }
    let scheme = base.split_once("://").map_or("https", |(scheme, _)| scheme);
    if let Some(rest) = location.strip_prefix("//") {
        return Ok(format!("{scheme}://{rest}"));
    }
    let origin = origin_of(base);
    if location.starts_with('/') {
        return Ok(format!("{origin}{location}"));
    }
    // Path-relative: resolve against the directory of the current URL.
    let base_path = base.split(['?', '#']).next().unwrap_or(base);
    let after_authority = base.split_once("://").map_or(base, |(_, rest)| rest);
    if !after_authority.contains('/') {
        return Ok(format!("{origin}/{location}"));
    }
    let directory = base_path.rsplit_once('/').map_or(origin, |(dir, _)| dir);
    Ok(format!("{directory}/{location}"))
}

/// `scheme://authority` of a URL, or `""` when there is no scheme.
fn origin_of(url: &str) -> &str {
    match url.find("://") {
        Some(scheme_end) => {
            let after = &url[scheme_end + 3..];
            let authority_len = after.find(['/', '?', '#']).unwrap_or(after.len());
            &url[..scheme_end + 3 + authority_len]
        }
        None => "",
    }
}

/// Check for updates on a background thread; the single message arrives on
/// the returned channel. Never blocks the caller.
#[must_use]
pub fn spawn_check(current_version: String, etag: Option<String>) -> Receiver<CheckOutcome> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let transport = UreqTransport::new(METADATA_TIMEOUT);
        let outcome = match fetch_latest_with(&transport, etag.as_deref()) {
            Ok(FetchOutcome::NotModified) => CheckOutcome::NotModified,
            Ok(FetchOutcome::Modified { release, etag }) => {
                if is_update(&current_version, &release.tag) {
                    CheckOutcome::Available {
                        release: Box::new(release),
                        etag,
                    }
                } else {
                    CheckOutcome::UpToDate { etag }
                }
            }
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
    signature_url: Option<String>,
    dest: PathBuf,
) -> Receiver<DownloadOutcome> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let transport = UreqTransport::new(DOWNLOAD_TIMEOUT);
        let outcome = download_verified_with(
            &transport,
            &asset,
            &checksums_url,
            signature_url.as_deref(),
            MINISIGN_PUBLIC_KEY,
            &dest,
            &|done, total| {
                let _ = sender.send(DownloadOutcome::Progress { done, total });
            },
        );
        let _ = sender.send(outcome);
    });
    receiver
}

/// Blocking download plus verification; see [`spawn_download`].
///
/// When a signing key is configured, the detached signature over the checksums
/// file must be present and valid; the download is refused (and deleted)
/// otherwise.
pub(super) fn download_verified_with(
    transport: &dyn HttpGet,
    asset: &AssetInfo,
    checksums_url: &str,
    signature_url: Option<&str>,
    public_key: &str,
    dest: &Path,
    progress: &dyn Fn(u64, Option<u64>),
) -> DownloadOutcome {
    if let Err(error) = download_file_with(transport, &asset.url, dest, progress) {
        let _ = std::fs::remove_file(dest);
        return DownloadOutcome::Failed(error.to_string());
    }
    let sums = match fetch_text_with(transport, checksums_url) {
        Ok(sums) => sums,
        Err(error) => {
            let _ = std::fs::remove_file(dest);
            return DownloadOutcome::Failed(error.to_string());
        }
    };
    if !public_key.is_empty() {
        let Some(signature_url) = signature_url else {
            let _ = std::fs::remove_file(dest);
            return DownloadOutcome::Failed(
                UpdateError::MissingSignature(SIGNATURE_FILE_NAME.to_owned()).to_string(),
            );
        };
        let signature = match fetch_text_with(transport, signature_url) {
            Ok(signature) => signature,
            Err(error) => {
                let _ = std::fs::remove_file(dest);
                return DownloadOutcome::Failed(error.to_string());
            }
        };
        if let Err(error) = verify_minisign(sums.as_bytes(), &signature, public_key) {
            let _ = std::fs::remove_file(dest);
            return DownloadOutcome::Failed(error.to_string());
        }
    }
    verify_against_sums(&sums, &asset.name, dest)
}

/// Stream `url` to `dest`, reporting `(bytes_written, advertised_total)`.
///
/// The body is bounded by [`MAX_INSTALLER_BYTES`], both from the advertised
/// `Content-Length` and while streaming, so a hostile or broken endpoint
/// cannot fill the disk.
fn download_file_with(
    transport: &dyn HttpGet,
    url: &str,
    dest: &Path,
    progress: &dyn Fn(u64, Option<u64>),
) -> Result<(), UpdateError> {
    let response = get_following_redirects(transport, url, "", None)?;
    if !(200..300).contains(&response.status()) {
        return Err(UpdateError::Network(format!(
            "installer request returned HTTP {}",
            response.status()
        )));
    }
    let total = response.content_length();
    if total.is_some_and(|total| total > MAX_INSTALLER_BYTES) {
        return Err(too_large());
    }
    let mut reader = response.into_body();
    let mut file = open_destination(dest)?;
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

/// Open `dest` for writing without following an existing path.
///
/// The download directory (`~/Downloads` or the temp dir) is user-writable, so
/// another process could pre-create `dest` as a symlink to a file the user
/// cares about and the download would then truncate the target. Any leftover
/// is removed first and the file is created exclusively, so the write always
/// lands in a fresh regular file; a non-removable obstacle becomes an error
/// rather than a follow-up write.
pub(super) fn open_destination(dest: &Path) -> Result<std::fs::File, UpdateError> {
    match std::fs::remove_file(dest) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(UpdateError::Io(error.to_string())),
    }
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest)
        .map_err(|error| UpdateError::Io(error.to_string()))
}

/// Error for a download that exceeds [`MAX_INSTALLER_BYTES`].
fn too_large() -> UpdateError {
    UpdateError::Response(format!(
        "installer larger than {} MiB",
        MAX_INSTALLER_BYTES / (1024 * 1024)
    ))
}

/// Fetch a small text body (used for [`CHECKSUMS_FILE_NAME`] and its
/// signature), bounded by [`MAX_CHECKSUMS_BYTES`] and validated like every
/// other update URL.
fn fetch_text_with(transport: &dyn HttpGet, url: &str) -> Result<String, UpdateError> {
    if url.is_empty() {
        return Err(UpdateError::MissingChecksum(CHECKSUMS_FILE_NAME.to_owned()));
    }
    let response = get_following_redirects(transport, url, "", None)?;
    if !(200..300).contains(&response.status()) {
        return Err(UpdateError::Network(format!(
            "checksums request returned HTTP {}",
            response.status()
        )));
    }
    let bytes = read_limited(response.into_body(), MAX_CHECKSUMS_BYTES)?;
    String::from_utf8(bytes).map_err(|error| UpdateError::Response(error.to_string()))
}
