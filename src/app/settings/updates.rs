//! Updates tab: running version, build hash and the update check.
//!
//! Split out of `settings.rs`; the updater state machine itself lives in
//! `crate::app::update_ui`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use eframe::egui::{self, RichText};
use fluent_bundle::FluentArgs;

use crate::app::Viewer;
use crate::app::update_ui::{CheckState, DownloadState};
use crate::app::workspace::human_size_u64;
use crate::build_info;
use crate::i18n;
use crate::theme::Palette;
use crate::update;

use super::UpdateAction;

impl Viewer {
    /// Version, build hash and the update check.
    pub(super) fn updates_tab(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        ctx: &egui::Context,
    ) {
        let mut version_args = FluentArgs::new();
        version_args.set("version", build_info::VERSION.to_owned());
        let version_label = self.i18n.render("update-current-version", &version_args);
        let mut build_args = FluentArgs::new();
        build_args.set("hash", build_info::build_hash_short());
        let build_label = self.i18n.render("update-current-build", &build_args);
        let mut tooltip_args = FluentArgs::new();
        tooltip_args.set("commit", build_info::commit_short());
        tooltip_args.set("hash", build_info::sha256_hex(build_info::GIT_COMMIT));
        let build_tooltip = self.i18n.render("update-build-tooltip", &tooltip_args);
        let auto_label = self.i18n.text("update-auto");
        let check_label = self.i18n.text("update-check");
        let checking_label = self.i18n.text("update-checking");

        ui.add_space(10.0);
        ui.label(RichText::new(&version_label).strong().color(palette.text));
        ui.label(
            RichText::new(&build_label)
                .monospace()
                .size(12.0)
                .color(palette.muted),
        )
        .on_hover_text(build_tooltip);

        ui.add_space(10.0);
        ui.checkbox(&mut self.update_auto, &auto_label);

        let checking = matches!(self.check, CheckState::Running { .. });
        let downloading = matches!(self.download, DownloadState::Running { .. });
        let mut action = None;

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !checking && !downloading,
                    egui::Button::new(check_label.as_str()),
                )
                .clicked()
            {
                action = Some(UpdateAction::Check);
            }
            if checking {
                ui.spinner();
                ui.label(
                    RichText::new(checking_label.as_str())
                        .size(12.0)
                        .color(palette.muted),
                );
            }
        });

        ui.add_space(10.0);
        if action.is_none() {
            action = self.update_status(ui, palette);
        }

        if !checking && let Some(last) = self.update_last_check {
            let when = relative_time(&self.i18n, last, update::now_unix());
            let mut args = FluentArgs::new();
            args.set("when", when);
            ui.add_space(8.0);
            ui.label(
                RichText::new(self.i18n.render("update-last-checked", &args))
                    .size(11.5)
                    .color(palette.muted),
            );
        }

        match action {
            Some(UpdateAction::Check) => self.start_check(true),
            Some(UpdateAction::Install) => self.start_install(),
            Some(UpdateAction::Launch) => self.launch_ready(ctx),
            Some(UpdateAction::Skip) => {
                if let Some(release) = self.update_release.take() {
                    self.update_skipped = release.tag;
                }
                self.update_error = None;
            }
            Some(UpdateAction::OpenPage) => {
                let _ = update::open_release_page(&update::releases_page_url());
            }
            None => {}
        }
    }

    /// The status body of the Updates tab and any action its buttons ask for.
    ///
    /// One state is shown at a time, in priority order: an in-flight download,
    /// a verified installer, an error, an offered release, "up to date", or
    /// "never checked".
    fn update_status(&self, ui: &mut egui::Ui, palette: &Palette) -> Option<UpdateAction> {
        let downloading_label = self.i18n.text("update-downloading");
        let up_to_date_label = self.i18n.text("update-up-to-date");
        let never_checked_label = self.i18n.text("update-never-checked");
        let retry_label = self.i18n.text("update-retry");
        let open_page_label = self.i18n.text("update-open-page");
        let ready_label = self.i18n.text("update-ready");
        let verified_label = self.i18n.text("update-verified");
        let skip_label = self.i18n.text("update-skip");
        let macos_hint = self.i18n.text("update-macos-hint");
        let install_label = self.install_label();
        let mut action = None;

        if let Some((done, total)) = self.update_download {
            download_progress(ui, palette, done, total, &downloading_label);
        } else if let Some((path, _digest)) = self.update_ready.clone() {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            );
            ui.label(
                RichText::new(ready_label.as_str())
                    .strong()
                    .color(palette.text),
            );
            ui.label(RichText::new(name).size(12.0).color(palette.muted));
            ui.label(
                RichText::new(verified_label.as_str())
                    .size(12.0)
                    .color(palette.muted),
            );
            ui.add_space(6.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(install_label.as_str()).color(palette.on_accent),
                    )
                    .fill(palette.accent),
                )
                .clicked()
            {
                action = Some(UpdateAction::Launch);
            }
        } else if let Some(reason) = self.update_error.clone() {
            ui.label(RichText::new(reason).color(palette.danger));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(retry_label.as_str()).clicked() {
                    action = Some(UpdateAction::Check);
                }
                if ui.button(open_page_label.as_str()).clicked() {
                    action = Some(UpdateAction::OpenPage);
                }
            });
        } else if let Some(release) = self.update_release.clone() {
            let mut release_args = FluentArgs::new();
            release_args.set("version", release.tag.clone());
            let headline = self.i18n.render("update-available", &release_args);
            ui.label(RichText::new(headline).strong().color(palette.text));
            if !self.update_summary.is_empty() {
                ui.label(
                    RichText::new(self.update_summary.clone())
                        .size(12.0)
                        .color(palette.muted),
                );
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new(install_label.as_str()).color(palette.on_accent),
                        )
                        .fill(palette.accent),
                    )
                    .clicked()
                {
                    action = Some(UpdateAction::Install);
                }
                if ui.button(skip_label.as_str()).clicked() {
                    action = Some(UpdateAction::Skip);
                }
            });
            if cfg!(target_os = "macos") {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(macos_hint.as_str())
                        .size(12.0)
                        .color(palette.muted),
                );
            }
        } else if self.update_up_to_date == Some(true) {
            ui.label(RichText::new(up_to_date_label.as_str()).color(palette.muted));
        } else if self.update_last_check.is_none() {
            ui.label(RichText::new(never_checked_label.as_str()).color(palette.muted));
        }
        action
    }

    /// What the install button should say on this platform.
    ///
    /// macOS deliberately only downloads the `.dmg` (the project ships
    /// unsigned builds and never self-updates the `/Applications` bundle), so
    /// the button never promises an install there.
    fn install_label(&self) -> String {
        if cfg!(target_os = "macos") {
            self.i18n.text("update-download-dmg")
        } else {
            self.i18n.text("update-install")
        }
    }
}

/// Download progress: a labelled bar with the bytes written so far.
fn download_progress(
    ui: &mut egui::Ui,
    palette: &Palette,
    done: u64,
    total: Option<u64>,
    label: &str,
) {
    let fraction = total
        .filter(|total| *total > 0)
        .map_or(0.0, |total| (done as f32 / total as f32).clamp(0.0, 1.0));
    let detail = match total {
        Some(total) => format!("{} / {}", human_size_u64(done), human_size_u64(total)),
        None => human_size_u64(done),
    };
    ui.label(RichText::new(label).color(palette.text));
    ui.add(
        egui::ProgressBar::new(fraction)
            .text(detail)
            .desired_width(260.0),
    );
}

/// Human, timezone-free age of `then` (Unix seconds) relative to `now`.
fn relative_time(i18n: &i18n::I18n, then: u64, now: u64) -> String {
    let seconds = now.saturating_sub(then);
    if seconds < 60 {
        i18n.text("time-just-now")
    } else if seconds < 3_600 {
        i18n.count("time-minutes-ago", seconds / 60)
    } else if seconds < 86_400 {
        i18n.count("time-hours-ago", seconds / 3_600)
    } else {
        i18n.count("time-days-ago", seconds / 86_400)
    }
}
