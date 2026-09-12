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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, TextEdit, TextStyle};
use fluent_bundle::FluentArgs;

use crate::i18n::I18n;
use crate::icons;
use crate::parser::{Capture, CaptureLimits, Part, PartText};

use crate::splash::{self, ScanView};
use crate::theme::{self, Palette};
use crate::worker::{Event, Worker};

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

/// Whether the in-module search bar is hidden, shown, or waiting for focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FindState {
    /// Hidden.
    Closed,
    /// Visible.
    Open,
    /// Visible and asking for keyboard focus this frame.
    Focus,
}

impl FindState {
    /// Whether the search bar should be drawn.
    const fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

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
        };

        if let Some(argument) = std::env::args_os().nth(1) {
            viewer.start_scan(PathBuf::from(argument));
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

    /// Recompute which lines contain the find query.
    fn recompute_matches(&mut self) {
        let needle = self.find.trim().to_owned();
        let mut found = Vec::new();
        if !needle.is_empty() {
            if let Some(body) = &self.body {
                for (row, &start) in self.line_starts.iter().enumerate() {
                    let end = self
                        .line_starts
                        .get(row + 1)
                        .copied()
                        .unwrap_or(body.text.len());
                    if find_case_insensitive(&body.text[start..end], &needle, 0).is_some() {
                        found.push(row);
                    }
                }
            }
        }
        self.matches = found;
        self.match_cursor = 0;
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
            });
        });
    }

    /// Top bar: the open capture, the rail toggle and the actions that need a home.
    ///
    /// The product name is deliberately absent: the operating system already
    /// draws it in the window title bar. There is no in-window menu either,
    /// because on macOS a menu belongs in the system bar.
    fn header(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let open_label = self.i18n.text("button-open");
        let toggle_hint = self.i18n.text("button-toggle-rail");
        let no_capture = self.i18n.text("label-no-capture");
        let name = self.source.as_deref().map_or_else(
            || no_capture.clone(),
            |path| {
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                )
            },
        );

        let mut open = false;
        let mut toggle_rail = false;
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            toggle_rail = icons::icon_button(
                ui,
                &palette,
                icons::Glyph::PanelLeft,
                self.rail_open,
                true,
                &toggle_hint,
            );
            ui.add_space(4.0);
            ui.label(RichText::new(name).size(12.5).color(palette.muted));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                if ui
                    .add(egui::Button::new(open_label).corner_radius(8.0))
                    .clicked()
                {
                    open = true;
                }
            });
        });

        if toggle_rail {
            self.rail_open = !self.rail_open;
        }
        if open {
            self.open_dialog();
        }
    }

    fn module_rail(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let modules_label = self.i18n.text("label-modules");
        let filter_hint = self.i18n.text("hint-filter");
        let clear_hint = self.i18n.text("button-clear");
        let no_matches = self.i18n.text("empty-filter");
        let total = self.capture.as_ref().map_or(0, |capture| capture.len());
        let counter = self
            .i18n
            .progress("label-counter", units(self.visible.len()), units(total));

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&modules_label).size(15.0).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&counter).size(12.0).color(palette.muted));
            });
        });
        ui.add_space(6.0);

        if search_field(ui, &palette, &mut self.filter, &filter_hint, &clear_hint) {
            self.refresh_visible();
        }
        ui.add_space(6.0);

        if self.visible.is_empty() && !self.filter.trim().is_empty() {
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(no_matches).size(12.5).color(palette.muted));
            });
            return;
        }

        // `show_rows` expects a height *without* item spacing; it adds the
        // spacing itself, so passing a padded height here breaks alignment.
        let row_height = ui.text_style_height(&TextStyle::Body);
        let mut chosen = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, row_height, self.visible.len(), |ui, range| {
                for row in range {
                    let index = self.visible[row];
                    let Some(capture) = &self.capture else { break };
                    let part = &capture.parts()[index];
                    let ordinal = if part.ordinal() == 0 {
                        String::new()
                    } else {
                        self.i18n
                            .count("label-copy-suffix", units(part.ordinal() + 1))
                    };
                    let flag = if part.is_readable() { "" } else { "  ⚠" };
                    let label = format!("{}{ordinal}{flag}", part.label());
                    let selected = self.selected == Some(index);
                    let text = if part.is_readable() {
                        RichText::new(label)
                    } else {
                        RichText::new(label).color(palette.danger)
                    };
                    if ui.selectable_label(selected, text).clicked() {
                        chosen = Some(index);
                    }
                }
            });

        if let Some(index) = chosen {
            self.select(index);
        }
    }

    fn reading_surface(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let empty_selection = self.i18n.text("empty-selection");
        let Some(index) = self.selected else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(empty_selection).color(palette.muted));
            });
            return;
        };

        let (label, compressed, readable, fault) = {
            let Some(capture) = &self.capture else {
                return;
            };
            let part = &capture.parts()[index];
            (
                part.label().to_owned(),
                part.compressed_len(),
                part.is_readable(),
                part.fault().map(str::to_owned),
            )
        };

        let mut size_args = FluentArgs::new();
        size_args.set("size", human_size(compressed));
        let compressed_label = self.i18n.render("label-compressed", &size_args);
        let save_as = self.i18n.text("button-save-as");
        let copy_label = self.i18n.text("button-copy");
        let line_numbers = self.i18n.text("label-line-numbers");
        let find_hint = self.i18n.text("button-find");
        let unreadable = self.i18n.text("module-unreadable");
        let decoding = self.i18n.text("status-decoding-inline");

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&label).size(16.0).strong());
            ui.label(
                RichText::new(compressed_label)
                    .size(12.0)
                    .color(palette.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(save_as).clicked() {
                    self.save_body();
                }
                if ui.button(copy_label).clicked() {
                    self.copy_body(ui.ctx());
                }
                ui.toggle_value(&mut self.gutter, line_numbers.as_str());
                if icons::icon_button(
                    ui,
                    &palette,
                    icons::Glyph::Magnifier,
                    self.find_state.is_open(),
                    true,
                    &find_hint,
                ) {
                    self.find_state = if self.find_state.is_open() {
                        FindState::Closed
                    } else {
                        FindState::Focus
                    };
                }
            });
        });
        ui.add_space(2.0);

        if !readable {
            let message = fault.as_deref().unwrap_or(unreadable.as_str());
            ui.label(RichText::new(message).color(palette.danger));
            return;
        }

        if self.find_state.is_open() {
            self.find_bar(ui);
        }

        if self.body.is_none() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(decoding).color(palette.muted));
            });
            return;
        }

        self.text_view(ui);
    }

    /// In-module search: one rounded card holding the magnifier, an inline
    /// field, the match counter and compact icon controls.
    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let hint = self.i18n.text("hint-find");
        let previous_hint = self.i18n.text("button-previous");
        let next_hint = self.i18n.text("button-next");
        let clear_hint = self.i18n.text("button-clear");

        let total = self.matches.len();
        let position = if total == 0 { 0 } else { self.match_cursor + 1 };
        let counter = self
            .i18n
            .progress("label-find-counter", units(position), units(total));
        let has_match = total > 0;

        let mut step = 0i32;
        let mut clear = false;
        let mut close = false;

        let frame = egui::Frame::NONE
            .fill(palette.card)
            .stroke(egui::Stroke::new(1.0, palette.card_hover))
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(8, 4));

        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;

                let controls = 26.0 * 3.0 + 6.0 * 4.0 + 52.0;
                let width = (ui.available_width() - controls).max(120.0);
                let response = ui.add(
                    TextEdit::singleline(&mut self.find)
                        .frame(egui::Frame::NONE)
                        .hint_text(hint)
                        .desired_width(width)
                        .margin(egui::Margin::symmetric(2, 3)),
                );
                let submitted =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                if self.find_state == FindState::Focus || submitted {
                    response.request_focus();
                    self.find_state = FindState::Open;
                }
                if response.changed() {
                    self.recompute_matches();
                }
                if submitted {
                    step = if ui.input(|input| input.modifiers.shift) {
                        -1
                    } else {
                        1
                    };
                }

                ui.label(RichText::new(&counter).size(12.0).color(palette.muted));
                if icons::icon_button(
                    ui,
                    &palette,
                    icons::Glyph::ChevronUp,
                    false,
                    has_match,
                    &previous_hint,
                ) {
                    step = -1;
                }
                if icons::icon_button(
                    ui,
                    &palette,
                    icons::Glyph::ChevronDown,
                    false,
                    has_match,
                    &next_hint,
                ) {
                    step = 1;
                }
                if icons::icon_button(ui, &palette, icons::Glyph::Close, false, true, &clear_hint) {
                    if self.find.is_empty() {
                        close = true;
                    } else {
                        clear = true;
                    }
                }
            });
        });

        if step != 0 && total > 0 {
            self.match_cursor = if step > 0 {
                (self.match_cursor + 1) % total
            } else {
                self.match_cursor.checked_sub(1).unwrap_or(total - 1)
            };
            self.scroll_to_line = self.matches.get(self.match_cursor).copied();
        }
        if clear {
            self.find.clear();
            self.recompute_matches();
        }
        if close {
            self.find_state = FindState::Closed;
        }
        ui.add_space(8.0);
    }

    fn text_view(&mut self, ui: &mut egui::Ui) {
        let jump = self.scroll_to_line.take();
        let gutter = self.gutter;
        let query = self.find.trim().to_owned();
        let font = TextStyle::Monospace.resolve(ui.style());
        let text_color = ui.visuals().text_color();
        let gutter_color = ui.visuals().weak_text_color();
        let highlight = ui.visuals().selection.bg_fill;
        let spacing = ui.spacing().item_spacing.y;
        // Height without spacing: `show_rows` adds the spacing itself.
        let row_height = ui.text_style_height(&TextStyle::Monospace);
        // Widest line across the whole module, so the horizontal scrollbar does
        // not flicker as different rows enter and leave the viewport.
        let char_width = ui.ctx().fonts_mut(|fonts| fonts.glyph_width(&font, '0'));
        let gutter_chars = if gutter { 8 } else { 0 };
        let content_width = (gutter_chars + self.max_line_chars) as f32 * char_width;

        let (text, line_starts) = match &self.body {
            Some(body) => (body.text.as_str(), &self.line_starts),
            None => return,
        };
        let lines = line_starts.len();

        let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
        if let Some(row) = jump {
            area = area.vertical_scroll_offset(row as f32 * (row_height + spacing));
        }
        area.show_rows(ui, row_height, lines, |ui, range| {
            ui.set_min_width(content_width);
            for row in range {
                let start = line_starts[row];
                let end = line_starts.get(row + 1).copied().unwrap_or(text.len());
                let raw = &text[start..end];
                let line = raw.strip_suffix('\n').unwrap_or(raw);
                let line = line.strip_suffix('\r').unwrap_or(line);

                let mut job = egui::text::LayoutJob::default();
                if gutter {
                    job.append(
                        &format!("{:>6}  ", row + 1),
                        0.0,
                        egui::TextFormat::simple(font.clone(), gutter_color),
                    );
                }
                append_highlighted(&mut job, line, &query, &font, text_color, highlight);
                // `Ui::label` overrides `job.wrap` with the Ui's own wrap mode, so
                // the row must be told to extend: otherwise a long line wraps,
                // grows past `row_height` and breaks the virtualised scroll.
                ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
            }
        });
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.last_time = ui.input(|input| input.time);
        self.drain();
        self.handle_dropped(ui.ctx());
        self.handle_shortcuts(ui);
        self.advance_stage();

        let palette = theme::current(ui.ctx());
        let file_hovered = ui.input(|input| !input.raw.hovered_files.is_empty());
        let footer_frame =
            egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(10, 3));
        egui::Panel::bottom("footer")
            .frame(footer_frame)
            .show(ui, |ui| self.footer_bar(ui));

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
        if self.busy.is_some() || self.stage == Stage::Scanning {
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

/// A comfortable, full-width search field on a rounded card.
///
/// Draws a magnifier, a borderless text field and a clear button that only
/// appears while there is something to clear. Returns `true` when the text
/// changed, whether by typing or by pressing clear.
fn search_field(
    ui: &mut egui::Ui,
    palette: &Palette,
    text: &mut String,
    hint: &str,
    clear_hint: &str,
) -> bool {
    let frame = egui::Frame::NONE
        .fill(palette.card)
        .stroke(egui::Stroke::new(1.0, palette.card_hover))
        .corner_radius(9.0)
        .inner_margin(egui::Margin::symmetric(10, 5));

    let mut changed = false;
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            let show_clear = !text.is_empty();
            let reserve = if show_clear { 32.0 } else { 0.0 };
            let width = (ui.available_width() - reserve).max(80.0);
            let response = ui.add(
                TextEdit::singleline(text)
                    .frame(egui::Frame::NONE)
                    .hint_text(hint.to_owned())
                    .desired_width(width)
                    .margin(egui::Margin::symmetric(2, 4)),
            );
            if response.changed() {
                changed = true;
            }
            if show_clear
                && icons::icon_button(ui, palette, icons::Glyph::Close, false, true, clear_hint)
            {
                text.clear();
                changed = true;
            }
        });
    });
    changed
}

/// Byte offsets at which each line starts.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (offset, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(offset + 1);
        }
    }
    starts
}

/// Length in characters of the longest line, ignoring line endings.
///
/// The text view multiplies this by the monospace advance to know how wide the
/// scrollable content is, independently of which rows happen to be on screen.
fn max_line_length(text: &str, starts: &[usize]) -> usize {
    let mut longest = 0;
    for (row, &start) in starts.iter().enumerate() {
        let end = starts.get(row + 1).copied().unwrap_or(text.len());
        let raw = &text[start..end];
        let line = raw.strip_suffix('\n').unwrap_or(raw);
        let line = line.strip_suffix('\r').unwrap_or(line);
        longest = longest.max(line.chars().count());
    }
    longest
}

/// Find `needle` in `hay` from `from`, ignoring ASCII case.
fn find_case_insensitive(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let haystack = hay.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    let last = haystack.len() - needle.len();
    if from > last {
        return None;
    }
    (from..=last)
        .find(|&offset| haystack[offset..offset + needle.len()].eq_ignore_ascii_case(needle))
}

/// Append one line to a layout job, marking every match of `query`.
fn append_highlighted(
    job: &mut egui::text::LayoutJob,
    line: &str,
    query: &str,
    font: &FontId,
    color: Color32,
    highlight: Color32,
) {
    let base = egui::TextFormat::simple(font.clone(), color);
    if query.is_empty() {
        job.append(line, 0.0, base);
        return;
    }
    let marked = egui::TextFormat {
        font_id: font.clone(),
        color,
        background: highlight,
        ..Default::default()
    };

    let mut cursor = 0usize;
    while let Some(offset) = find_case_insensitive(line, query, cursor) {
        if offset > cursor {
            job.append(&line[cursor..offset], 0.0, base.clone());
        }
        let end = offset + query.len();
        job.append(&line[offset..end], 0.0, marked.clone());
        cursor = end;
    }
    if cursor < line.len() {
        job.append(&line[cursor..], 0.0, base);
    }
}

/// Turn a module label into a safe file name fragment.
fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let trimmed =
        cleaned.trim_matches(|character: char| character == '_' || character.is_whitespace());
    if trimmed.is_empty() {
        "module".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Convert a collection length into the unsigned type used by messages.
fn units(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Clamp an unsigned quantity into the signed number Fluent expects.
fn to_number(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn human_size(bytes: usize) -> String {
    human_size_u64(units(bytes))
}

fn human_size_u64(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_starts_covers_every_line() {
        assert_eq!(line_starts(""), vec![0]);
        assert_eq!(line_starts("a\nb\n"), vec![0, 2, 4]);
        assert_eq!(line_starts("a\nb"), vec![0, 2]);
    }

    #[test]
    fn max_line_length_ignores_line_endings() {
        let text = "a\r\nlonger line\nx";
        assert_eq!(max_line_length(text, &line_starts(text)), 11);
    }

    #[test]
    fn case_insensitive_search_finds_all_offsets() {
        assert_eq!(find_case_insensitive("Ether1", "eth", 0), Some(0));
        assert_eq!(find_case_insensitive("aXbXc", "x", 0), Some(1));
        assert_eq!(find_case_insensitive("aXbXc", "x", 2), Some(3));
        assert_eq!(find_case_insensitive("abc", "z", 0), None);
    }

    #[test]
    fn sanitize_removes_path_separators() {
        assert_eq!(sanitize("/ip/firewall/filter"), "ip_firewall_filter");
        assert_eq!(sanitize("log"), "log");
    }
}
