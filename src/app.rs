//! Desktop shell: welcome, home, opening animation and the module workspace.
//!
//! The interface thread never parses or inflates anything itself. It renders the
//! current stage and delegates all heavy work to [`Worker`]. Custom-painted
//! screens read the [`Palette`] that matches the operating system theme.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

mod update_ui;
mod workspace;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align, Color32, Layout, RichText};
use fluent_bundle::FluentArgs;

use crate::i18n::I18n;
use crate::parser::{Capture, CaptureLimits, Part, PartText};
use crate::update::{self, ReleaseInfo};

use crate::splash::{self, ScanView};
use crate::theme::{self, Palette};
use crate::worker::{Event, Worker};

use self::update_ui::{CheckState, DownloadState};
use self::workspace::{
    FindState, human_size_u64, line_starts, max_line_length, sanitize, to_number, units,
};

/// Minimum seconds the opening animation stays on screen.
const SCAN_MIN_SECONDS: f64 = 2.4;
/// Extra seconds the animation holds after indexing, so it never cuts abruptly.
const SCAN_HOLD_SECONDS: f64 = 1.1;

/// Which screen the shell is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// First-run welcome with the Start action.
    Welcome,
    /// Where the user chooses how to get a capture.
    Home,
    /// Opening animation while the capture is read and indexed.
    Scanning,
    /// Module list and text output.
    Workspace,
}

/// eframe storage key for the automatic update check toggle.
const UPDATE_AUTO_KEY: &str = "update.auto_check";
/// eframe storage key for the last completed update check (Unix seconds).
const UPDATE_LAST_KEY: &str = "update.last_check";
/// eframe storage key for the release tag the user chose to skip.
const UPDATE_SKIPPED_KEY: &str = "update.skipped_tag";

/// A transient message shown in the footer.
struct Notice {
    message: String,
    is_error: bool,
}

/// Application state.
pub struct Viewer {
    worker: Worker,
    limits: CaptureLimits,
    i18n: I18n,
    stage: Stage,
    last_time: f64,
    fade_at: Option<f64>,

    // Opening animation.
    scan_started: f64,
    scan_min_until: Option<f64>,
    read_received: u64,
    read_total: Option<u64>,
    home_error: Option<String>,

    // Capture.
    capture: Option<Arc<Capture>>,
    source: Option<PathBuf>,
    filter: String,
    visible: Vec<usize>,
    selected: Option<usize>,
    body: Option<PartText>,
    line_starts: Vec<usize>,
    max_line_chars: usize,
    find: String,
    matches: Vec<usize>,
    match_cursor: usize,
    scroll_to_line: Option<usize>,
    pending_expand: Option<usize>,
    busy: Option<String>,
    notice: Option<Notice>,
    gutter: bool,
    rail_open: bool,
    find_state: FindState,
    /// A captured panic waiting for the user to dismiss it, if any.
    panic_report: Option<crate::panic::PanicReport>,

    // In-app updater (the only network path; see `crate::update`).
    /// Daily automatic checks enabled (persisted).
    update_auto: bool,
    /// Last completed check in Unix seconds (persisted).
    update_last_check: Option<u64>,
    /// Release tag the user chose to skip (persisted).
    update_skipped: String,
    /// Release tag dismissed with "Later" (session only).
    update_dismissed: String,
    /// Latest release newer than the running app, if one was found.
    update_release: Option<ReleaseInfo>,
    /// Verified installer waiting for the handoff, with the SHA-256 digest it
    /// was verified against (re-checked immediately before use).
    update_ready: Option<(PathBuf, String)>,
    /// Last update failure to show (manual checks only).
    update_error: Option<String>,
    /// Background check state machine (at most one check runs at a time).
    check: CheckState,
    /// Background download state machine.
    download: DownloadState,
    /// Download progress: bytes written and advertised total, if known.
    update_download: Option<(u64, Option<u64>)>,
}

impl Viewer {
    /// Build the viewer and open a capture passed on the command line, if any.
    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let mut viewer = Self {
            worker: Worker::spawn(),
            limits: CaptureLimits::default(),
            i18n: I18n::detected(),
            stage: Stage::Welcome,
            last_time: 0.0,
            fade_at: None,
            scan_started: 0.0,
            scan_min_until: None,
            read_received: 0,
            read_total: None,
            home_error: None,
            capture: None,
            source: None,
            filter: String::new(),
            visible: Vec::new(),
            selected: None,
            body: None,
            line_starts: Vec::new(),
            max_line_chars: 0,
            find: String::new(),
            matches: Vec::new(),
            match_cursor: 0,
            scroll_to_line: None,
            pending_expand: None,
            busy: None,
            notice: None,
            gutter: true,
            rail_open: true,
            find_state: FindState::Closed,
            panic_report: None,
            update_auto: true,
            update_last_check: None,
            update_skipped: String::new(),
            update_dismissed: String::new(),
            update_release: None,
            update_ready: None,
            update_error: None,
            check: CheckState::Idle,
            download: DownloadState::Idle,
            update_download: None,
        };

        viewer.restore_update_prefs(cc.storage);

        if let Some(argument) = std::env::args_os().nth(1) {
            viewer.start_scan(PathBuf::from(argument));
        }
        if update::should_auto_check(
            viewer.update_auto,
            viewer.update_last_check,
            update::now_unix(),
        ) {
            viewer.start_check(false);
        }
        viewer
    }

    fn set_notice(&mut self, message: String, is_error: bool) {
        self.notice = Some(Notice { message, is_error });
    }

    /// Format a message that takes a `$path` and a `$reason`.
    fn message_with_path(&self, id: &str, path: &Path, reason: &str) -> String {
        let mut args = FluentArgs::new();
        args.set("path", path.display().to_string());
        args.set("reason", reason);
        self.i18n.render(id, &args)
    }

    /// Enter the opening animation and queue the capture for reading.
    fn start_scan(&mut self, path: PathBuf) {
        self.stage = Stage::Scanning;
        self.fade_at = Some(self.last_time);
        self.home_error = None;
        self.capture = None;
        self.body = None;
        self.line_starts.clear();
        self.max_line_chars = 0;
        self.matches.clear();
        self.visible.clear();
        self.selected = None;
        self.pending_expand = None;
        self.busy = None;
        self.notice = None;
        self.read_received = 0;
        self.read_total = None;
        self.scan_started = self.last_time;
        self.scan_min_until = Some(self.last_time + SCAN_MIN_SECONDS);
        self.source = Some(path.clone());
        self.worker.index(path);
    }

    /// Ask the worker to expand a module.
    fn request_expand(&mut self, index: usize) {
        let Some(capture) = self.capture.clone() else {
            return;
        };
        self.pending_expand = Some(index);
        self.busy = Some(self.i18n.text("status-decoding"));
        self.worker.expand(capture, index, self.limits);
    }

    /// Select a module and expand it.
    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        self.request_expand(index);
    }

    /// Apply every event that arrived since the last frame.
    fn drain(&mut self) {
        loop {
            let Some(event) = self.worker.poll() else {
                return;
            };
            match event {
                Event::Reading { received, total } => {
                    self.read_received = received;
                    self.read_total = total;
                }
                Event::Indexed { path, capture } => {
                    let modules = units(capture.len());
                    self.capture = Some(capture);
                    self.source = Some(path);
                    self.scan_min_until = Some(
                        (self.scan_started + SCAN_MIN_SECONDS)
                            .max(self.last_time + SCAN_HOLD_SECONDS),
                    );
                    let message = self.i18n.count("status-indexed", modules);
                    self.set_notice(message, false);
                }
                Event::IndexFailed { path, reason } => {
                    self.stage = Stage::Home;
                    self.fade_at = Some(self.last_time);
                    self.capture = None;
                    self.set_notice(String::new(), false);
                    let message = self.message_with_path("error-open", &path, &reason);
                    self.home_error = Some(message);
                }
                Event::Expanded { index, text } => {
                    if self.pending_expand == Some(index) {
                        self.pending_expand = None;
                        self.busy = None;
                        self.set_body(text);
                    }
                }
                Event::ExpandFailed { index, reason } => {
                    if self.pending_expand == Some(index) {
                        self.pending_expand = None;
                        self.busy = None;
                        self.body = None;
                        let mut args = FluentArgs::new();
                        args.set("index", to_number(units(index)));
                        args.set("reason", reason);
                        let message = self.i18n.render("error-module", &args);
                        self.set_notice(message, true);
                    }
                }
            }
        }
    }

    fn set_body(&mut self, text: PartText) {
        self.line_starts = line_starts(&text.text);
        self.max_line_chars = max_line_length(&text.text, &self.line_starts);
        self.body = Some(text);
        self.scroll_to_line = Some(0);
        self.recompute_matches();
    }

    fn refresh_visible(&mut self) {
        let Some(capture) = &self.capture else {
            self.visible.clear();
            return;
        };
        let needle = self.filter.trim().to_lowercase();
        self.visible = capture
            .parts()
            .iter()
            .enumerate()
            .filter(|(_, part)| needle.is_empty() || part.label().to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect();
    }

    /// Move from the animation to the workspace once indexing is done.
    fn advance_stage(&mut self) {
        if self.stage != Stage::Scanning {
            return;
        }
        let Some(until) = self.scan_min_until else {
            return;
        };
        if self.capture.is_none() || self.last_time < until {
            return;
        }

        self.stage = Stage::Workspace;
        self.fade_at = Some(self.last_time);
        self.refresh_visible();
        if let Some(first) = self
            .capture
            .as_ref()
            .and_then(|capture| capture.parts().iter().position(Part::is_readable))
        {
            self.select(first);
        }
    }

    fn open_dialog(&mut self) {
        let filter = self.i18n.text("dialog-filter-name");
        let title = self.i18n.text("dialog-open-title");
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(filter, &["rif"])
            .set_title(title)
            .pick_file()
        {
            self.start_scan(path);
        }
    }

    fn suggested_file_name(&self) -> String {
        let stem = self
            .source
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(
                || "capture".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
        let name = self.capture.as_ref().zip(self.selected).map_or_else(
            || "module".to_owned(),
            |(capture, index)| sanitize(capture.parts()[index].label()),
        );
        format!("{stem}__{name}.txt")
    }

    fn copy_body(&mut self, ctx: &egui::Context) {
        if let Some(body) = &self.body {
            let bytes = units(body.text.len());
            let text = body.text.clone();
            ctx.copy_text(text);
            let message = self.i18n.count("status-copied", bytes);
            self.set_notice(message, false);
        }
    }

    fn save_body(&mut self) {
        let default_name = self.suggested_file_name();
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        let outcome = match self.body.as_ref() {
            Some(body) => {
                std::fs::write(&path, body.text.as_bytes()).map_err(|error| error.to_string())
            }
            None => return,
        };
        match outcome {
            Ok(()) => {
                let mut args = FluentArgs::new();
                args.set("path", path.display().to_string());
                let message = self.i18n.render("status-saved", &args);
                self.set_notice(message, false);
            }
            Err(error) => {
                let mut args = FluentArgs::new();
                args.set("reason", error);
                let message = self.i18n.render("error-save", &args);
                self.set_notice(message, true);
            }
        }
    }

    fn handle_dropped(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if let Some(path) = dropped.into_iter().next() {
            self.start_scan(path);
        }
    }

    fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let (open_requested, save_requested, find_requested, escape) = ui.input(|input| {
            let command = input.modifiers.command;
            (
                command && input.key_pressed(egui::Key::O),
                command && input.key_pressed(egui::Key::S),
                command && input.key_pressed(egui::Key::F),
                input.key_pressed(egui::Key::Escape),
            )
        });
        if open_requested && self.stage != Stage::Welcome {
            self.open_dialog();
        }
        if save_requested && self.body.is_some() {
            self.save_body();
        }
        if find_requested && self.stage == Stage::Workspace && self.selected.is_some() {
            self.find_state = FindState::Focus;
        }
        if escape {
            self.find_state = FindState::Closed;
        }
    }

    fn footer_bar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let ready = self.i18n.text("status-ready");
        let mut args = FluentArgs::new();
        args.set("version", env!("CARGO_PKG_VERSION"));
        args.set("os", theme::platform_name());
        args.set("arch", std::env::consts::ARCH);
        let footer = self.i18n.render("footer-build", &args);
        let check_label = self.i18n.text("update-check");
        let auto_label = self.i18n.text("update-auto");

        let mut check_requested = false;
        ui.horizontal(|ui| {
            if let Some(busy) = &self.busy {
                ui.spinner();
                ui.label(RichText::new(busy).size(12.0).color(palette.muted));
            } else if let Some(notice) = &self.notice {
                if notice.message.is_empty() {
                    ui.label(RichText::new(&ready).size(12.0).color(palette.muted));
                } else {
                    let color = if notice.is_error {
                        palette.danger
                    } else {
                        palette.muted
                    };
                    ui.label(RichText::new(&notice.message).size(12.0).color(color));
                }
            } else {
                ui.label(RichText::new(&ready).size(12.0).color(palette.muted));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&footer).size(12.0).color(palette.muted));
                if ui.small_button(&check_label).clicked() {
                    check_requested = true;
                }
                ui.checkbox(&mut self.update_auto, &auto_label);
            });
        });
        if check_requested {
            self.start_check(true);
        }
    }

    /// Show the last-resort panic dialog while a captured report is pending.
    fn panic_dialog(&mut self, ctx: &egui::Context) {
        let mut dismissed = false;
        if let Some(report) = &self.panic_report {
            dismissed = crate::panic::show_dialog(ctx, report);
        }
        if dismissed {
            self.panic_report = None;
        }
    }

    fn draw_fade(&self, ctx: &egui::Context, palette: &Palette) {
        let Some(start) = self.fade_at else {
            return;
        };
        let elapsed = (self.last_time - start) as f32;
        let duration = 0.35;
        if elapsed >= duration {
            return;
        }
        let alpha = ((1.0 - elapsed / duration) * 255.0).clamp(0.0, 255.0) as u8;
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("stage-fade"),
        ));
        painter.rect_filled(
            ctx.content_rect(),
            0.0,
            Color32::from_rgba_unmultiplied(
                palette.window.r(),
                palette.window.g(),
                palette.window.b(),
                alpha,
            ),
        );
    }
}

impl eframe::App for Viewer {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let auto = if self.update_auto { "1" } else { "0" };
        storage.set_string(UPDATE_AUTO_KEY, auto.to_owned());
        if let Some(last) = self.update_last_check {
            storage.set_string(UPDATE_LAST_KEY, last.to_string());
        }
        storage.set_string(UPDATE_SKIPPED_KEY, self.update_skipped.clone());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.last_time = ui.input(|input| input.time);
        self.drain();
        self.poll_update();
        self.handle_dropped(ui.ctx());
        self.handle_shortcuts(ui);
        self.advance_stage();
        if let Some(report) = crate::panic::take_report() {
            self.panic_report = Some(report);
        }

        let palette = theme::current(ui.ctx());
        let file_hovered = ui.input(|input| !input.raw.hovered_files.is_empty());
        let footer_frame =
            egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(10, 3));
        egui::Panel::bottom("footer")
            .frame(footer_frame)
            .show(ui, |ui| self.footer_bar(ui));
        if self.update_banner_visible() {
            self.update_banner(ui);
        }

        match self.stage {
            Stage::Welcome => {
                let mut started = false;
                egui::CentralPanel::default().show(ui, |ui| {
                    theme::paint_background(
                        ui.painter(),
                        ui.available_rect_before_wrap(),
                        &palette,
                    );
                    started = splash::welcome(ui, &self.i18n, &palette);
                });
                if started {
                    self.stage = Stage::Home;
                    self.fade_at = Some(self.last_time);
                }
            }
            Stage::Home => {
                let error = self.home_error.clone();
                let mut chosen = false;
                egui::CentralPanel::default().show(ui, |ui| {
                    theme::paint_background(
                        ui.painter(),
                        ui.available_rect_before_wrap(),
                        &palette,
                    );
                    chosen = splash::home(ui, &self.i18n, &palette, file_hovered);
                    if let Some(message) = &error {
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(message).size(13.0).color(palette.danger));
                        });
                    }
                });
                if chosen {
                    self.open_dialog();
                }
            }
            Stage::Scanning => {
                let (phase, detail, progress) = self.scan_labels();
                egui::CentralPanel::default().show(ui, |ui| {
                    theme::paint_background(
                        ui.painter(),
                        ui.available_rect_before_wrap(),
                        &palette,
                    );
                    let view = ScanView {
                        elapsed: (self.last_time - self.scan_started) as f32,
                        progress,
                        phase: &phase,
                        detail: &detail,
                    };
                    splash::scanning(ui, &view, &palette);
                });
            }
            Stage::Workspace => {
                egui::Panel::top("header").show(ui, |ui| self.header(ui));
                if self.rail_open {
                    egui::Panel::left("module_rail")
                        .resizable(true)
                        .default_size(340.0)
                        .size_range(220.0..=620.0)
                        .show(ui, |ui| self.module_rail(ui));
                }
                let content_frame = egui::Frame::central_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(14, 6));
                egui::CentralPanel::default()
                    .frame(content_frame)
                    .show(ui, |ui| self.reading_surface(ui));
            }
        }

        self.draw_fade(ui.ctx(), &palette);
        self.panic_dialog(ui.ctx());
        if self.busy.is_some()
            || self.stage == Stage::Scanning
            || !matches!(self.check, CheckState::Idle)
            || !matches!(self.download, DownloadState::Idle)
        {
            ui.ctx().request_repaint();
        }
    }
}

impl Viewer {
    /// Phase text, detail text and progress fraction for the scanning view.
    fn scan_labels(&self) -> (String, String, Option<f32>) {
        if let Some(capture) = &self.capture {
            return (
                self.i18n.text("phase-preparing"),
                self.i18n
                    .count("detail-modules-found", units(capture.len())),
                Some(1.0),
            );
        }

        let progress = self
            .read_total
            .filter(|total| *total > 0)
            .map(|total| (self.read_received as f32 / total as f32).clamp(0.0, 1.0));

        match (self.read_total, progress) {
            (Some(total), Some(fraction)) if self.read_received >= total => (
                self.i18n.text("phase-indexing"),
                self.i18n.text("detail-indexing"),
                Some(fraction),
            ),
            (Some(total), Some(fraction)) => {
                let mut args = FluentArgs::new();
                args.set("done", human_size_u64(self.read_received));
                args.set("total", human_size_u64(total));
                (
                    self.i18n.text("phase-reading"),
                    self.i18n.render("detail-read-of", &args),
                    Some(fraction),
                )
            }
            _ => {
                let mut args = FluentArgs::new();
                args.set("size", human_size_u64(self.read_received));
                (
                    self.i18n.text("phase-reading"),
                    self.i18n.render("detail-read", &args),
                    None,
                )
            }
        }
    }
}
