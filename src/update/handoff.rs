//! Hand-off of a verified installer to the operating system.
//!
//! Split out of `update.rs`. Windows launches the NSIS installer, a Linux
//! AppImage replaces itself atomically, and everything else is opened with
//! the desktop handler.
use std::path::{Path, PathBuf};

use super::*;

#[cfg(target_os = "linux")]
use std::os::unix::fs::PermissionsExt as _;

/// Re-hash `path` and compare it with the digest captured at download time.
///
/// The file exists between verification and use, and a local process with
/// write access to the download directory could swap it in that window. The
/// file is deleted on a mismatch so nothing unverified can be launched.
pub(super) fn reverify(path: &Path, expected_hex: &str) -> Result<(), UpdateError> {
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
pub(super) fn install_appimage_file(current: &Path, new_file: &Path) -> Result<(), UpdateError> {
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
pub(super) fn check_appimage_target(current: &Path) -> Result<(), UpdateError> {
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
