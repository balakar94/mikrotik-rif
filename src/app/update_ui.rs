//! In-app updater presentation: background check/download and banner.
//!
//! This module owns the updater state machines and the dismissable banner
//! shown above the content. It never blocks the workspace; at most one
//! check and one download run at a time.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::path::Path;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Align, Layout, RichText};
use fluent_bundle::FluentArgs;

use crate::theme::{self, Palette};
use crate::update::{self, CheckOutcome, DownloadOutcome};

use super::Viewer;
use super::workspace::human_size_u64;
use super::{UPDATE_AUTO_KEY, UPDATE_LAST_KEY, UPDATE_SKIPPED_KEY};

/// What the user picked in the update banner; applied after drawing it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BannerAction {
    /// Download the available release.
    Install,
    /// Dismiss the banner until the next relevant state.
    Later,
    /// Never offer this release tag again.
    Skip,
    /// Hand the verified installer to the operating system.
    LaunchReady,
}

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
    }

    /// Start a background update check, unless one is already running.
    pub(crate) fn start_check(&mut self, manual: bool) {
        if matches!(self.check, CheckState::Running { .. }) {
            return;
        }
        self.update_error = None;
        let rx = update::spawn_check(env!("CARGO_PKG_VERSION").to_owned());
        self.check = CheckState::Running { manual, rx };
    }

    /// Drain finished background update events into the displayed state.
    pub(crate) fn poll_update(&mut self) {
        let check_event = match &self.check {
            CheckState::Idle => None,
            CheckState::Running { rx, .. } => match rx.try_recv() {
                Ok(outcome) => Some(Some(outcome)),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
            },
        };
        // A fresh check supersedes a stale error once it starts.
        let manual = matches!(&self.check, CheckState::Running { manual: true, .. });
        match check_event {
            Some(Some(CheckOutcome::Available(release))) => {
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
                if release.tag != self.update_skipped {
                    self.update_release = Some(*release);
                    self.update_ready = None;
                    self.update_download = None;
                }
            }
            Some(Some(CheckOutcome::UpToDate)) => {
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
                self.update_release = None;
            }
            Some(Some(CheckOutcome::Failed(reason))) => {
                self.check = CheckState::Idle;
                self.update_last_check = Some(update::now_unix());
                if manual {
                    self.update_error = Some(reason);
                }
            }
            Some(None) => {
                self.check = CheckState::Idle;
            }
            None => {}
        }

        let download_event = match &self.download {
            DownloadState::Idle => None,
            DownloadState::Running { rx } => match rx.try_recv() {
                Ok(outcome) => Some(Some(outcome)),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(None),
            },
        };
        match download_event {
            Some(Some(DownloadOutcome::Progress { done, total })) => {
                self.update_download = Some((done, total));
            }
            Some(Some(DownloadOutcome::Done { path, expected_hex })) => {
                self.download = DownloadState::Idle;
                self.update_download = None;
                self.update_ready = Some((path, expected_hex));
            }
            Some(Some(DownloadOutcome::Failed(reason))) => {
                self.download = DownloadState::Idle;
                self.update_download = None;
                self.update_error = Some(reason);
            }
            Some(None) => {
                self.download = DownloadState::Idle;
                self.update_download = None;
            }
            None => {}
        }
        if matches!(self.check, CheckState::Running { .. }) {
            self.update_error = None;
        }
    }

    /// Begin downloading the available release (or open its page when no
    /// asset matches this platform).
    fn start_install(&mut self) {
        let Some(release) = self.update_release.clone() else {
            return;
        };
        let Some(asset) = update::select_asset_for_current(&release.assets) else {
            if update::open_release_page(&release.page_url).is_err() {
                self.update_error = Some(release.page_url.clone());
            }
            self.update_dismissed.clone_from(&release.tag);
            return;
        };
        let dest = update::asset_dest(&asset.name);
        let sums = update::checksums_url(&release).unwrap_or_default();
        self.update_download = Some((0, None));
        let rx = update::spawn_download(asset.clone(), sums, dest);
        self.download = DownloadState::Running { rx };
    }

    /// Whether the update banner has anything to show right now.
    pub(crate) fn update_banner_visible(&self) -> bool {
        if !matches!(self.check, CheckState::Idle) || self.update_error.is_some() {
            return true;
        }
        if !matches!(self.download, DownloadState::Idle) || self.update_ready.is_some() {
            return true;
        }
        self.update_release.as_ref().is_some_and(|release| {
            release.tag != self.update_skipped && release.tag != self.update_dismissed
        })
    }

    /// Dismissable, single-purpose update banner shown above the content.
    ///
    /// It only ever shows one state at a time (checking, available,
    /// downloading, ready, or error) and never blocks the workspace.
    pub(crate) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let now_label = self.i18n.text("update-now");
        let later_label = self.i18n.text("update-later");
        let skip_label = self.i18n.text("update-skip");
        let check_label = self.i18n.text("update-check");
        let available = self.update_release.as_ref().map(|release| {
            let mut version_args = FluentArgs::new();
            version_args.set("version", release.tag.clone());
            let mut headline = self.i18n.render("update-available", &version_args);
            let title = release
                .name
                .as_deref()
                .map(str::trim)
                .filter(|title| !title.is_empty() && *title != release.tag);
            if let Some(title) = title {
                headline.push_str(" — ");
                headline.push_str(title);
            }
            (headline, update::summarize_notes(release.notes()))
        });

        let mut action: Option<BannerAction> = None;
        let mut close_requested = false;
        let checking = matches!(self.check, CheckState::Running { .. });

        egui::Panel::top("update-banner").show(ui, |ui| {
            ui.horizontal(|ui| {
                if checking {
                    ui.spinner();
                    ui.label(&check_label);
                } else if let Some(reason) = &self.update_error {
                    Self::banner_error(ui, &palette, reason, &later_label, &mut action);
                } else if let Some((done, total)) = self.update_download {
                    Self::banner_download(ui, done, total);
                } else if let Some((path, _digest)) = &self.update_ready {
                    Self::banner_ready(ui, path, &now_label, &later_label, &mut action);
                } else if let Some((headline, notes)) = &available {
                    Self::banner_available(
                        ui,
                        &palette,
                        headline,
                        notes,
                        &now_label,
                        &later_label,
                        &skip_label,
                        &mut action,
                    );
                }
            });
        });

        match action {
            Some(BannerAction::Install) => self.start_install(),
            Some(BannerAction::Skip) => {
                if let Some(release) = self.update_release.take() {
                    self.update_skipped = release.tag;
                }
                self.update_error = None;
            }
            Some(BannerAction::Later) => {
                if let Some(release) = &self.update_release {
                    self.update_dismissed.clone_from(&release.tag);
                }
                self.update_ready = None;
                self.update_error = None;
            }
            Some(BannerAction::LaunchReady) => {
                if let Some((path, digest)) = self.update_ready.clone() {
                    // On Linux a running AppImage replaces itself instead of
                    // going through the desktop handler; anywhere else (or for
                    // any other package kind) the regular handoff applies.
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
                }
            }
            None => {}
        }
        if close_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Regular installer handoff: launch/open the verified file and update the
    /// banner state from the outcome.
    fn finish_handoff(
        &mut self,
        path: &std::path::Path,
        expected_hex: &str,
        close_requested: &mut bool,
    ) {
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

    /// Banner row for a failed check or download, with a dismiss button.
    fn banner_error(
        ui: &mut egui::Ui,
        palette: &Palette,
        reason: &str,
        later_label: &str,
        action: &mut Option<BannerAction>,
    ) {
        ui.label(RichText::new(reason).color(palette.danger));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button(later_label).clicked() {
                *action = Some(BannerAction::Later);
            }
        });
    }

    /// Banner row for an in-flight download: spinner plus byte progress.
    fn banner_download(ui: &mut egui::Ui, done: u64, total: Option<u64>) {
        let fraction = total
            .filter(|total| *total > 0)
            .map_or(0.0, |total| (done as f32 / total as f32).clamp(0.0, 1.0));
        let detail = match total {
            Some(total) => format!("{} / {}", human_size_u64(done), human_size_u64(total)),
            None => human_size_u64(done),
        };
        ui.spinner();
        ui.add(
            egui::ProgressBar::new(fraction)
                .text(detail)
                .desired_width(220.0),
        );
    }

    /// Banner row for a verified installer waiting for the handoff.
    fn banner_ready(
        ui: &mut egui::Ui,
        path: &Path,
        now_label: &str,
        later_label: &str,
        action: &mut Option<BannerAction>,
    ) {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        ui.label(&name);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button(later_label).clicked() {
                *action = Some(BannerAction::Later);
            }
            if ui.small_button(now_label).clicked() {
                *action = Some(BannerAction::LaunchReady);
            }
        });
    }

    /// Banner row for a newer release: version, notes excerpt, three actions.
    #[allow(clippy::too_many_arguments)]
    fn banner_available(
        ui: &mut egui::Ui,
        palette: &Palette,
        headline: &str,
        notes: &str,
        now_label: &str,
        later_label: &str,
        skip_label: &str,
        action: &mut Option<BannerAction>,
    ) {
        ui.label(RichText::new(headline).strong());
        if !notes.is_empty() {
            ui.label(RichText::new(notes).size(12.0).color(palette.muted));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button(skip_label).clicked() {
                *action = Some(BannerAction::Skip);
            }
            if ui.small_button(later_label).clicked() {
                *action = Some(BannerAction::Later);
            }
            if ui.small_button(now_label).clicked() {
                *action = Some(BannerAction::Install);
            }
        });
    }
}
