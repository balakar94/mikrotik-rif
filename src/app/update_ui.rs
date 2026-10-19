//! In-app updater state: background check/download machines and hand-off.
//!
//! The updater's presentation now lives in the Settings screen
//! (`super::settings`), but the state it reads stays here so a check that
//! finishes while the user is elsewhere in the app is never lost. At most one
//! check and one download run at a time, and none of them block the interface
//! thread.
//!
//! The single source of truth for the network rules is [`crate::update`]; this
//! module only tracks the lifecycle of those background jobs. [`Updater`]
//! holds that lifecycle, and its [`Updater::poll`] transitions are unit-tested
//! without a window or a network: the tests inject outcomes through the same
//! channels the worker threads use.

use std::path::Path;
use std::sync::mpsc::Receiver;

use eframe::egui;

use crate::update::{self, CheckOutcome, DownloadOutcome, ReleaseInfo};

use super::Viewer;
use super::{UPDATE_AUTO_KEY, UPDATE_ETAG_KEY, UPDATE_LAST_KEY, UPDATE_SKIPPED_KEY};

/// Background update-check state: idle, or waiting on the worker thread.
///
/// A two-variant enum instead of bare booleans, so the running receiver and
/// the manual/automatic origin travel together and cannot drift apart.
#[derive(Default)]
pub(crate) enum CheckState {
    /// No check in flight.
    #[default]
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
#[derive(Default)]
pub(crate) enum DownloadState {
    /// No download in flight.
    #[default]
    Idle,
    /// A download thread is streaming; progress arrives on the channel.
    Running {
        /// Channel carrying progress and the final outcome.
        rx: Receiver<DownloadOutcome>,
    },
}

/// Everything the Settings screen shows about updates, plus the job handles.
///
/// Separated from [`Viewer`] so the transition logic in [`Self::poll`] can be
/// exercised without an `eframe` context: tests build an `Updater`, put a
/// receiver in [`Self::check`] or [`Self::download`], send outcomes through
/// it and assert on the visible fields.
pub(crate) struct Updater {
    /// Daily automatic checks enabled (persisted).
    pub(crate) auto: bool,
    /// Last completed check in Unix seconds (persisted).
    pub(crate) last_check: Option<u64>,
    /// Release tag the user chose to skip (persisted).
    pub(crate) skipped: String,
    /// Latest release newer than the running app, if one was found.
    pub(crate) release: Option<ReleaseInfo>,
    /// Verified installer waiting for the handoff, with the SHA-256 digest it
    /// was verified against (re-checked immediately before use).
    pub(crate) ready: Option<(std::path::PathBuf, String)>,
    /// Last update failure to show (manual checks and downloads only).
    pub(crate) error: Option<String>,
    /// Whether the last completed check found the app up to date (`None` while
    /// a check runs, after a failure, or before the first one ever runs).
    pub(crate) up_to_date: Option<bool>,
    /// ETag of the last release-metadata response (persisted), so a repeated
    /// check can answer `304` instead of consuming the API rate limit.
    pub(crate) etag: Option<String>,
    /// Background check state machine (at most one check runs at a time).
    pub(crate) check: CheckState,
    /// Background download state machine.
    pub(crate) download: DownloadState,
    /// Download progress: bytes written and advertised total, if known.
    pub(crate) progress: Option<(u64, Option<u64>)>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            // Automatic checks are on for first-run users, matching the
            // pre-rendered checkbox in Settings.
            auto: true,
            last_check: None,
            skipped: String::new(),
            release: None,
            ready: None,
            error: None,
            up_to_date: None,
            etag: None,
            check: CheckState::Idle,
            download: DownloadState::Idle,
            progress: None,
        }
    }
}

impl Updater {
    /// Drain finished background events into the displayed state.
    ///
    /// Called every frame regardless of which screen is visible, so an
    /// automatic check or a download started from Settings keeps making
    /// progress while the user reads a capture.
    pub(crate) fn poll(&mut self) {
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
                self.last_check = Some(update::now_unix());
                self.up_to_date = Some(false);
                self.etag = etag;
                // A skipped tag stays quiet for the automatic check, but an
                // explicit "Check for updates" still surfaces it.
                if manual || release.tag != self.skipped {
                    self.release = Some(*release);
                    self.ready = None;
                    self.progress = None;
                }
            }
            Some(CheckOutcome::UpToDate { etag }) => {
                self.check = CheckState::Idle;
                self.last_check = Some(update::now_unix());
                self.up_to_date = Some(true);
                self.etag = etag;
                self.release = None;
            }
            Some(CheckOutcome::NotModified) => {
                // Nothing changed since the last check: keep the current state
                // (which may be an update still waiting to be installed).
                self.check = CheckState::Idle;
                self.last_check = Some(update::now_unix());
            }
            Some(CheckOutcome::Failed(reason)) => {
                self.check = CheckState::Idle;
                self.last_check = Some(update::now_unix());
                self.up_to_date = None;
                if manual {
                    self.error = Some(reason);
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
            self.progress = progress;
        }
        match terminal {
            Some(DownloadOutcome::Done { path, expected_hex }) => {
                self.download = DownloadState::Idle;
                self.progress = None;
                self.ready = Some((path, expected_hex));
            }
            Some(DownloadOutcome::Failed(reason)) => {
                self.download = DownloadState::Idle;
                self.progress = None;
                self.error = Some(reason);
            }
            Some(DownloadOutcome::Progress { .. }) | None => {}
        }

        if matches!(self.check, CheckState::Running { .. }) {
            self.error = None;
        }
    }

    /// Whether a background check is in flight.
    #[must_use]
    pub(crate) fn is_checking(&self) -> bool {
        matches!(self.check, CheckState::Running { .. })
    }

    /// Whether a background download is in flight.
    #[must_use]
    pub(crate) fn is_downloading(&self) -> bool {
        matches!(self.download, DownloadState::Running { .. })
    }

    /// Whether an update is currently offered.
    ///
    /// A skipped release is not stored here by the automatic check, so this is
    /// simply "there is something in Settings to act on right now".
    #[must_use]
    pub(crate) fn available(&self) -> bool {
        self.release.is_some()
    }

    /// Release tag of the offered update, if any.
    #[must_use]
    pub(crate) fn version(&self) -> Option<&str> {
        self.release.as_ref().map(|release| release.tag.as_str())
    }
}

impl Viewer {
    /// Load the persisted updater preferences, if eframe storage is available.
    pub(crate) fn restore_update_prefs(&mut self, storage: Option<&dyn eframe::Storage>) {
        let Some(storage) = storage else {
            return;
        };
        if let Some(auto) = storage.get_string(UPDATE_AUTO_KEY) {
            self.updater.auto = auto != "0";
        }
        self.updater.last_check = storage
            .get_string(UPDATE_LAST_KEY)
            .and_then(|value| value.parse().ok());
        self.updater.skipped = storage.get_string(UPDATE_SKIPPED_KEY).unwrap_or_default();
        self.updater.etag = storage
            .get_string(UPDATE_ETAG_KEY)
            .filter(|value| !value.is_empty());
    }

    /// Start a background update check, unless one is already running.
    ///
    /// Sends the stored ETag so an unchanged release answers `304`, which does
    /// not count against the GitHub API rate limit.
    pub(crate) fn start_check(&mut self, manual: bool) {
        if self.updater.is_checking() {
            return;
        }
        self.updater.error = None;
        self.updater.up_to_date = None;
        let rx = update::spawn_check(
            env!("CARGO_PKG_VERSION").to_owned(),
            self.updater.etag.clone(),
        );
        self.updater.check = CheckState::Running { manual, rx };
    }

    /// Begin downloading the available release (or open its page when no
    /// asset matches this platform).
    pub(crate) fn start_install(&mut self) {
        let Some(release) = self.updater.release.clone() else {
            return;
        };
        let Some(asset) = update::select_asset_for_current(&release.assets) else {
            if update::open_release_page(&release.page_url).is_err() {
                self.updater.error = Some(
                    "no installer matches this platform; open the release page manually".to_owned(),
                );
            }
            return;
        };
        let Some(sums) = update::checksums_url(&release) else {
            // Without a checksum file there is nothing to verify against, so
            // the download is never started in the first place.
            self.updater.error = Some(
                update::UpdateError::MissingChecksum(update::CHECKSUMS_FILE_NAME.to_owned())
                    .to_string(),
            );
            return;
        };
        let dest = update::asset_dest(&asset.name);
        let signature = update::signature_url(&release);
        self.updater.progress = Some((0, None));
        let rx = update::spawn_download(asset.clone(), sums, signature, dest);
        self.updater.download = DownloadState::Running { rx };
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
        let Some((path, digest)) = self.updater.ready.take() else {
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
                self.updater.error = Some(error.to_string());
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
                self.updater.ready = None;
                self.updater.release = None;
            }
            Err(error) => {
                self.updater.error = Some(error.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Sender};

    use super::*;

    fn release(tag: &str) -> ReleaseInfo {
        ReleaseInfo {
            tag: tag.to_owned(),
            name: None,
            body: None,
            page_url: format!("https://example.com/{tag}"),
            assets: Vec::new(),
        }
    }

    /// Put the updater in the "check running" state with a channel the test
    /// controls, as `start_check` would after spawning its thread.
    fn checking(updater: &mut Updater, manual: bool) -> Sender<CheckOutcome> {
        let (tx, rx) = mpsc::channel();
        updater.check = CheckState::Running { manual, rx };
        tx
    }

    /// Put the updater in the "download running" state with a test channel.
    fn downloading(updater: &mut Updater) -> Sender<DownloadOutcome> {
        let (tx, rx) = mpsc::channel();
        updater.download = DownloadState::Running { rx };
        tx
    }

    #[test]
    fn an_available_release_is_offered_and_remembered() {
        let mut updater = Updater::default();
        let tx = checking(&mut updater, true);
        tx.send(CheckOutcome::Available {
            release: Box::new(release("v9.9.9")),
            etag: Some("\"abc\"".to_owned()),
        })
        .unwrap();

        updater.poll();

        assert!(!updater.is_checking());
        assert_eq!(updater.version(), Some("v9.9.9"));
        assert_eq!(updater.up_to_date, Some(false));
        assert_eq!(updater.etag.as_deref(), Some("\"abc\""));
        assert!(updater.last_check.is_some());
        assert!(updater.error.is_none());
    }

    #[test]
    fn an_automatic_check_keeps_a_skipped_release_quiet() {
        let mut updater = Updater {
            skipped: "v9.9.9".to_owned(),
            ..Updater::default()
        };
        let tx = checking(&mut updater, false);
        tx.send(CheckOutcome::Available {
            release: Box::new(release("v9.9.9")),
            etag: None,
        })
        .unwrap();

        updater.poll();

        assert!(!updater.available(), "a skipped tag must stay hidden");
        assert_eq!(updater.up_to_date, Some(false));
    }

    #[test]
    fn a_manual_check_surfaces_a_skipped_release() {
        let mut updater = Updater {
            skipped: "v9.9.9".to_owned(),
            ..Updater::default()
        };
        let tx = checking(&mut updater, true);
        tx.send(CheckOutcome::Available {
            release: Box::new(release("v9.9.9")),
            etag: None,
        })
        .unwrap();

        updater.poll();

        assert_eq!(updater.version(), Some("v9.9.9"));
    }

    #[test]
    fn a_new_release_clears_a_stale_installer_and_progress() {
        let mut updater = Updater {
            ready: Some((PathBuf::from("/tmp/setup.exe"), "digest".to_owned())),
            progress: Some((10, Some(100))),
            release: Some(release("v9.0.0")),
            ..Updater::default()
        };
        let tx = checking(&mut updater, false);
        tx.send(CheckOutcome::Available {
            release: Box::new(release("v9.9.9")),
            etag: None,
        })
        .unwrap();

        updater.poll();

        assert_eq!(updater.version(), Some("v9.9.9"));
        assert!(updater.ready.is_none(), "the old installer must be dropped");
        assert!(updater.progress.is_none());
    }

    #[test]
    fn an_up_to_date_answer_clears_the_offered_release() {
        let mut updater = Updater {
            release: Some(release("v9.9.9")),
            error: Some("old failure".to_owned()),
            ..Updater::default()
        };
        let tx = checking(&mut updater, true);
        tx.send(CheckOutcome::UpToDate {
            etag: Some("\"new\"".to_owned()),
        })
        .unwrap();

        updater.poll();

        assert!(!updater.available());
        assert_eq!(updater.up_to_date, Some(true));
        assert_eq!(updater.etag.as_deref(), Some("\"new\""));
        assert!(updater.last_check.is_some());
    }

    #[test]
    fn not_modified_keeps_the_pending_update_and_installer() {
        let mut updater = Updater {
            release: Some(release("v9.9.9")),
            ready: Some((PathBuf::from("/tmp/setup.exe"), "digest".to_owned())),
            etag: Some("\"same\"".to_owned()),
            ..Updater::default()
        };
        let tx = checking(&mut updater, false);
        tx.send(CheckOutcome::NotModified).unwrap();

        updater.poll();

        assert_eq!(updater.version(), Some("v9.9.9"));
        assert!(updater.ready.is_some());
        assert_eq!(updater.etag.as_deref(), Some("\"same\""));
        assert_eq!(updater.up_to_date, None);
        assert!(updater.last_check.is_some());
    }

    #[test]
    fn an_automatic_failure_stays_silent() {
        let mut updater = Updater::default();
        let tx = checking(&mut updater, false);
        tx.send(CheckOutcome::Failed("offline".to_owned())).unwrap();

        updater.poll();

        assert!(updater.error.is_none(), "automatic failures are quiet");
        assert_eq!(updater.up_to_date, None);
        assert!(updater.last_check.is_some());
    }

    #[test]
    fn a_manual_failure_is_shown() {
        let mut updater = Updater::default();
        let tx = checking(&mut updater, true);
        tx.send(CheckOutcome::Failed("offline".to_owned())).unwrap();

        updater.poll();

        assert_eq!(updater.error.as_deref(), Some("offline"));
        assert_eq!(updater.up_to_date, None);
    }

    #[test]
    fn a_dropped_check_channel_is_a_manual_failure() {
        let mut updater = Updater::default();
        let tx = checking(&mut updater, true);
        drop(tx);

        updater.poll();

        assert!(!updater.is_checking());
        let reason = updater.error.as_deref().unwrap_or_default();
        assert!(
            reason.contains("stopped unexpectedly"),
            "unexpected reason: {reason:?}"
        );
    }

    #[test]
    fn a_running_check_hides_a_previous_failure() {
        let mut updater = Updater {
            error: Some("old failure".to_owned()),
            ..Updater::default()
        };
        let _tx = checking(&mut updater, true);

        updater.poll();

        assert!(updater.error.is_none(), "a retry clears the old error");
    }

    #[test]
    fn download_progress_keeps_only_the_newest_sample() {
        let mut updater = Updater::default();
        let tx = downloading(&mut updater);
        for (done, total) in [(1, None), (20, Some(100)), (42, Some(100))] {
            tx.send(DownloadOutcome::Progress { done, total }).unwrap();
        }

        updater.poll();

        assert!(updater.is_downloading(), "no terminal event yet");
        assert_eq!(updater.progress, Some((42, Some(100))));
    }

    #[test]
    fn a_finished_download_becomes_a_ready_installer() {
        let mut updater = Updater {
            progress: Some((10, Some(100))),
            ..Updater::default()
        };
        let tx = downloading(&mut updater);
        tx.send(DownloadOutcome::Progress {
            done: 100,
            total: Some(100),
        })
        .unwrap();
        tx.send(DownloadOutcome::Done {
            path: PathBuf::from("/tmp/setup.exe"),
            expected_hex: "abc123".to_owned(),
        })
        .unwrap();

        updater.poll();

        assert!(!updater.is_downloading());
        assert!(updater.progress.is_none(), "the bar gives way to the offer");
        assert_eq!(
            updater.ready,
            Some((PathBuf::from("/tmp/setup.exe"), "abc123".to_owned()))
        );
    }

    #[test]
    fn a_failed_download_is_shown_and_clears_progress() {
        let mut updater = Updater {
            progress: Some((10, Some(100))),
            ..Updater::default()
        };
        let tx = downloading(&mut updater);
        tx.send(DownloadOutcome::Failed("checksum mismatch".to_owned()))
            .unwrap();

        updater.poll();

        assert!(!updater.is_downloading());
        assert!(updater.progress.is_none());
        assert_eq!(updater.error.as_deref(), Some("checksum mismatch"));
    }

    #[test]
    fn a_dropped_download_channel_is_a_failure() {
        let mut updater = Updater::default();
        let tx = downloading(&mut updater);
        drop(tx);

        updater.poll();

        assert!(!updater.is_downloading());
        let reason = updater.error.as_deref().unwrap_or_default();
        assert!(
            reason.contains("stopped unexpectedly"),
            "unexpected reason: {reason:?}"
        );
    }

    #[test]
    fn an_idle_updater_ignores_a_poll() {
        let mut updater = Updater::default();

        updater.poll();

        assert!(!updater.is_checking());
        assert!(!updater.is_downloading());
        assert!(!updater.available());
        assert!(updater.last_check.is_none());
        assert!(updater.error.is_none());
    }
}
