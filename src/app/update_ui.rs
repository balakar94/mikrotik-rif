//! In-app updater state: background check/download machines and hand-off.
//!
//! The updater's presentation now lives in the Settings screen
//! (`super::settings`), but the state it reads stays here so a check that
//! finishes while the user is elsewhere in the app is never lost. At most one
//! check and one download run at a time, and none of them block the interface
//! thread.
//!
//! The single source of truth for the network rules is [`crate::update`]; this
//! module only tracks the lifecycle of those background jobs.

use std::path::Path;
use std::sync::mpsc::Receiver;

use eframe::egui;

use crate::update::{self, CheckOutcome, DownloadOutcome};

use super::Viewer;
use super::{UPDATE_AUTO_KEY, UPDATE_ETAG_KEY, UPDATE_LAST_KEY, UPDATE_SKIPPED_KEY};

/// Background update-check state: idle, or waiting on the worker thread.
///
/// A two-variant enum instead of bare booleans, so the running receiver and
/// the manual/automatic origin travel together and cannot drift apart.
pub(crate) enum CheckState {
    /// No check in flight.
    Idle,
    /// A check thread is running; `manual` remembers its origin.
    Running {
        /// Whether the user triggered this check (failures are only shown then).
        manual: bool,
        /// Channel carrying the single outcome message.
        rx: Receiver<CheckOutcome>,
    },
}

/// Background download state: idle, or streaming on the worker thread.
pub(crate) enum DownloadState {
    /// No download in flight.
    Idle,
    /// A download thread is streaming; progress arrives on the channel.
    Running {
        /// Channel carrying progress and the final outcome.
        rx: Receiver<DownloadOutcome>,
    },
}

impl Viewer {
    /// Load the persisted updater preferences, if eframe storage is available.
    pub(crate) fn restore_update_prefs(&mut self, storage: Option<&dyn eframe::Storage>) {
        let Some(storage) = storage else {
            return;
        };
        if let Some(auto) = storage.get_string(UPDATE_AUTO_KEY) {
            self.update_auto = auto != "0";
        }
        self.update_last_check = storage
            .get_string(UPDATE_LAST_KEY)
            .and_then(|value| value.parse().ok());
        self.update_skipped = storage.get_string(UPDATE_SKIPPED_KEY).unwrap_or_default();
        self.update_etag = storage
            .get_string(UPDATE_ETAG_KEY)
            .filter(|value| !value.is_empty());
    }

    /// Start a background update check, unless one is already running.
    ///
    /// Sends the stored ETag so an unchanged release answers `304`, which does
    /// not count against the GitHub API rate limit.
    pub(crate) fn start_check(&mut self, manual: bool) {
        if matches!(self.check, CheckState::Running { .. }) {
            return;
        }
        self.update_error = None;
        self.update_up_to_date = None;
        let rx = update::spawn_check(
            env!("CARGO_PKG_VERSION").to_owned(),
            self.update_etag.clone(),
        );
        self.check = CheckState::Running { manual, rx };
    }

    /// Drain finished background update events into the displayed state.
    ///
    /// Called every frame regardless of which screen is visible, so an
    /// automatic check or a download started from Settings keeps making
    /// progress while the user reads a capture.
    pub(crate) fn poll_update(&mut self) {
        // A check produces exactly one message.
        let check_event = match &self.check {
            CheckState::Idle => None,
            CheckState::Running { rx, .. } => match rx.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(CheckOutcome::Failed(
                    "update check stopped unexpectedly".to_owned(),
                )),
            },
        };
        let manual = matches!(&self.check, CheckState::Running { manual: true, .. });
        match check_event {
            Some(CheckOutcome::Available { release, etag }) => {
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
                self.update_up_to_date = Some(false);
                self.update_etag = etag;
                // A skipped tag stays quiet for the automatic check, but an
                // explicit "Check for updates" still surfaces it.
                if manual || release.tag != self.update_skipped {
                    self.update_release = Some(*release);
                    self.update_ready = None;
                    self.update_download = None;
                }
            }
            Some(CheckOutcome::UpToDate { etag }) => {
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
                self.update_up_to_date = Some(true);
                self.update_etag = etag;
                self.update_release = None;
            }
            Some(CheckOutcome::NotModified) => {
                // Nothing changed since the last check: keep the current state
                // (which may be an update still waiting to be installed).
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
            }
            Some(CheckOutcome::Failed(reason)) => {
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
                self.update_up_to_date = None;
                if manual {
                    self.update_error = Some(reason);
                }
            }
            None => {}
        }

        // A download streams many progress messages. Drain the channel in one
        // pass and keep only the newest progress plus any terminal event, so a
        // large installer cannot leave the bar seconds behind the bytes on
        // disk (or grow the channel unboundedly).
        let (progress, terminal) = match &self.download {
            DownloadState::Idle => (None, None),
            DownloadState::Running { rx } => {
                let mut progress = None;
                let mut terminal = None;
                loop {
                    match rx.try_recv() {
                        Ok(DownloadOutcome::Progress { done, total }) => {
                            progress = Some((done, total));
                        }
                        Ok(outcome) => {
                            terminal = Some(outcome);
                            break;
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            terminal = Some(DownloadOutcome::Failed(
                                "update download stopped unexpectedly".to_owned(),
                            ));
                            break;
                        }
                    }
                }
                (progress, terminal)
            }
        };
        if progress.is_some() {
            self.update_download = progress;
        }
        match terminal {
            Some(DownloadOutcome::Done { path, expected_hex }) => {
                self.download = DownloadState::Idle;
                self.update_download = None;
                self.update_ready = Some((path, expected_hex));
            }
            Some(DownloadOutcome::Failed(reason)) => {
                self.download = DownloadState::Idle;
                self.update_download = None;
                self.update_error = Some(reason);
            }
            Some(DownloadOutcome::Progress { .. }) | None => {}
        }

        if matches!(self.check, CheckState::Running { .. }) {
            self.update_error = None;
        }
    }

    /// Whether an update is currently offered.
    ///
    /// A skipped release is not stored here by the automatic check, so this is
    /// simply "there is something in Settings to act on right now".
    pub(crate) fn update_available(&self) -> bool {
        self.update_release.is_some()
    }

    /// Release tag of the offered update, if any.
    pub(crate) fn update_version(&self) -> Option<&str> {
        self.update_release
            .as_ref()
            .map(|release| release.tag.as_str())
    }

    /// Begin downloading the available release (or open its page when no
    /// asset matches this platform).
    pub(crate) fn start_install(&mut self) {
        let Some(release) = self.update_release.clone() else {
            return;
        };
        let Some(asset) = update::select_asset_for_current(&release.assets) else {
            if update::open_release_page(&release.page_url).is_err() {
                self.update_error = Some(
                    "no installer matches this platform; open the release page manually".to_owned(),
                );
            }
            return;
        };
        let Some(sums) = update::checksums_url(&release) else {
            // Without a checksum file there is nothing to verify against, so
            // the download is never started in the first place.
            self.update_error = Some(
                update::UpdateError::MissingChecksum(update::CHECKSUMS_FILE_NAME.to_owned())
                    .to_string(),
            );
            return;
        };
        let dest = update::asset_dest(&asset.name);
        let signature = update::signature_url(&release);
        self.update_download = Some((0, None));
        let rx = update::spawn_download(asset.clone(), sums, signature, dest);
        self.download = DownloadState::Running { rx };
    }

    /// Hand a verified installer to the operating system.
    ///
    /// On Linux a running AppImage replaces itself instead of going through the
    /// desktop handler; anywhere else (or for any other package kind) the
    /// regular handoff applies. When the handoff ends the process (installer
    /// launched or image replaced), the window is asked to close.
    pub(crate) fn launch_ready(&mut self, ctx: &egui::Context) {
        // Consume the ready state so a second click cannot launch a second
        // installer while the first handoff is still starting.
        let Some((path, digest)) = self.update_ready.take() else {
            return;
        };
        let mut close_requested = false;
        #[cfg(target_os = "linux")]
        match update::maybe_self_replace(&path, &digest) {
            Some(Ok(update::HandoffAction::AppImageReplaced)) => {
                close_requested = true;
            }
            Some(Ok(_)) => {}
            Some(Err(error)) => {
                self.update_error = Some(error.to_string());
            }
            None => self.finish_handoff(&path, &digest, &mut close_requested),
        }
        #[cfg(not(target_os = "linux"))]
        self.finish_handoff(&path, &digest, &mut close_requested);

        if close_requested {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Regular installer handoff: launch/open the verified file and update the
    /// state from the outcome.
    fn finish_handoff(&mut self, path: &Path, expected_hex: &str, close_requested: &mut bool) {
        match update::perform_handoff(path, expected_hex) {
            Ok(
                update::HandoffAction::InstallerLaunched | update::HandoffAction::AppImageReplaced,
            ) => {
                *close_requested = true;
            }
            Ok(update::HandoffAction::PackageOpened) => {
                self.update_ready = None;
                self.update_release = None;
            }
            Err(error) => {
                self.update_error = Some(error.to_string());
            }
        }
    }
}
