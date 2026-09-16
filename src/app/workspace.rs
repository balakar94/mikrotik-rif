//! Module workspace: capture list, reading surface and in-module search.
//!
//! This module owns the workspace presentation: the top header, the module
//! rail, the reading surface and the find bar, plus the pure text helpers
//! used to index lines and to format sizes and file names.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::path::Path;

use eframe::egui::{self, Align, Color32, FontId, Layout, RichText, TextEdit, TextStyle};
use fluent_bundle::FluentArgs;

use crate::icons;
use crate::theme::{self, Palette};

use super::Viewer;

/// Whether the in-module search bar is hidden, shown, or waiting for focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindState {
    /// Hidden.
    Closed,
    /// Visible.
    Open,
    /// Visible and asking for keyboard focus this frame.
    Focus,
}

impl FindState {
    /// Whether the search bar should be drawn.
    pub(crate) const fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

impl Viewer {
    /// Top bar: the open capture, the rail toggle and the settings entry.
    ///
    /// The product name is deliberately absent: the operating system already
    /// draws it in the window title bar. There is no in-window menu either,
    /// because on macOS a menu belongs in the system bar.
    ///
    /// The open capture is shown as a clickable chip that also opens another
    /// file, so the action sits next to the context it acts on instead of in
    /// the opposite corner. The right edge is reserved for the settings gear,
    /// a stable anchor that also carries the update badge.
    pub(crate) fn header(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let open_label = self.i18n.text("button-open");
        let open_hint = self.i18n.text("button-open-capture");
        let toggle_hint = self.i18n.text("button-toggle-rail");
        let no_capture = self.i18n.text("label-no-capture");
        let settings_hint = self.settings_hint();
        let settings_badge = self.update_available();
        let source = self.source.clone();

        let mut open = false;
        let mut toggle_rail = false;
        let mut open_settings = false;
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            // Chevron points where the rail will go: `<` to collapse it,
            // `>` to bring it back.
            let rail_glyph = if self.rail_open {
                icons::Glyph::ChevronLeft
            } else {
                icons::Glyph::ChevronRight
            };
            toggle_rail =
                icons::icon_button(ui, &palette, rail_glyph, self.rail_open, true, &toggle_hint);
            ui.add_space(4.0);
            if let Some(path) = &source {
                let name = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                if file_chip(ui, &palette, &name, path, &open_hint).clicked() {
                    open = true;
                }
            } else {
                ui.label(RichText::new(no_capture).size(12.5).color(palette.muted));
                if ui.button(open_label).clicked() {
                    open = true;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add_space(6.0);
                if super::settings::gear_button(ui, &palette, settings_badge, &settings_hint) {
                    open_settings = true;
                }
            });
        });

        if toggle_rail {
            self.rail_open = !self.rail_open;
        }
        if open_settings {
            self.settings = Some(super::settings::SettingsTab::General);
        }
        if open {
            self.open_dialog();
        }
    }

    pub(crate) fn module_rail(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let modules_label = self.i18n.text("label-modules");
        let filter_hint = self.i18n.text("hint-filter");
        let clear_hint = self.i18n.text("button-clear");
        let no_matches = self.i18n.text("empty-filter");
        let clear_label = self.i18n.text("button-clear");
        let total = self.capture.as_ref().map_or(0, |capture| capture.len());
        // Translator note: `{ $position }` is the filtered count,
        // `{ $total }` is the total module count.
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
            let mut clear_filter = false;
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(no_matches).size(12.5).color(palette.muted));
                ui.add_space(8.0);
                if ui.button(clear_label).clicked() {
                    clear_filter = true;
                }
            });
            if clear_filter {
                self.filter.clear();
                self.refresh_visible();
            }
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

    pub(crate) fn reading_surface(&mut self, ui: &mut egui::Ui) {
        if self.selected.is_none() {
            self.show_no_selection(ui);
            return;
        }
        let index = self.selected.unwrap_or(0);
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
        let decoding = self.i18n.text("status-decoding-inline");

        self.reading_header(ui, &label, &compressed_label, readable);
        ui.add_space(2.0);

        if !readable {
            self.show_unreadable(ui, fault.as_deref());
            return;
        }

        if self.find_state.is_open() {
            self.find_bar(ui);
        }

        if self.body.is_none() {
            let palette = theme::current(ui.ctx());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(decoding).color(palette.muted));
            });
            return;
        }

        self.text_view(ui);
    }

    /// Placeholder when no module is selected: the usual hint, or a call to
    /// open another file when the capture holds nothing readable.
    fn show_no_selection(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let no_readable = self.capture.as_ref().is_some_and(|capture| {
            !capture.parts().is_empty() && capture.parts().iter().all(|part| !part.is_readable())
        });
        if !no_readable {
            let empty_selection = self.i18n.text("empty-selection");
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new(empty_selection).color(palette.muted));
            });
            return;
        }
        let message = self.i18n.text("empty-no-readable");
        let open_label = self.i18n.text("button-open");
        let mut open = false;
        ui.centered_and_justified(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(message).color(palette.muted));
                ui.add_space(8.0);
                if ui.button(open_label).clicked() {
                    open = true;
                }
            });
        });
        if open {
            self.open_dialog();
        }
    }

    /// Title row: truncated module name with full tooltip, compressed size and
    /// the copy/save/gutter/find controls on the right.
    fn reading_header(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        compressed_label: &str,
        readable: bool,
    ) {
        let palette = theme::current(ui.ctx());
        let save_as = self.i18n.text("button-save-as");
        let copy_label = self.i18n.text("button-copy");
        let line_numbers = self.i18n.text("label-line-numbers");
        let find_hint = self.i18n.text("button-find");

        ui.add_space(4.0);
        let short_title = ellipsize_title(label);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&short_title).size(16.0).strong())
                .on_hover_text(label);
            ui.label(
                RichText::new(compressed_label)
                    .size(12.0)
                    .color(palette.muted),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(readable, egui::Button::new(save_as))
                    .clicked()
                {
                    self.save_body();
                }
                if ui
                    .add_enabled(readable, egui::Button::new(copy_label))
                    .clicked()
                {
                    self.copy_body(ui.ctx());
                }
                // Gutter toggle: icon-only is hover-only (`on_hover_text` in
                // icons.rs reports no button role/name to assistive tech), so
                // show the localized "Line numbers" text as a real toggle
                // button when there is room, falling back to the icon in the
                // same spot on narrow widths. `Button::selectable` exposes
                // toggle semantics and the visible name to AT/keyboard focus.
                let gutter_clicked = if ui.available_width() > 360.0 {
                    ui.add(egui::Button::selectable(self.gutter, &line_numbers).small())
                        .on_hover_text(&line_numbers)
                        .clicked()
                } else if icons::icon_button(
                    ui,
                    &palette,
                    icons::Glyph::PanelLeft,
                    self.gutter,
                    true,
                    &line_numbers,
                ) {
                    true
                } else {
                    // Keep the accessible name perceivable even in icon mode:
                    // a focused screen-reader user still gets the tooltip text
                    // via the icon's hover text, while sighted keyboard users
                    // see the focus ring. The wide layout above is the fully
                    // perceivable variant.
                    false
                };
                if gutter_clicked {
                    self.gutter = !self.gutter;
                }
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
    }

    /// Danger banner naming the unreadable module plus a muted fault line.
    fn show_unreadable(&mut self, ui: &mut egui::Ui, fault: Option<&str>) {
        let palette = theme::current(ui.ctx());
        let unreadable = self.i18n.text("module-unreadable");
        ui.label(RichText::new(unreadable).color(palette.danger));
        if let Some(reason) = fault {
            ui.label(RichText::new(reason).size(12.0).color(palette.muted));
        }
    }

    /// In-module search: one rounded card holding the magnifier, an inline
    /// field, the match counter and compact icon controls.
    fn find_bar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let hint = self.i18n.text("hint-find");
        let previous_hint = self.i18n.text("button-previous");
        let next_hint = self.i18n.text("button-next");
        let clear_hint = self.i18n.text("button-clear");
        let keys_hint = self.i18n.text("hint-find-keys");

        let total = self.matches.len();
        let position = if total == 0 { 0 } else { self.match_cursor + 1 };
        // Translator note: `{ $position }` is the current match,
        // `{ $total }` is the total match count.
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
        ui.label(RichText::new(keys_hint).size(11.0).color(palette.muted));
        ui.add_space(8.0);
    }

    fn text_view(&mut self, ui: &mut egui::Ui) {
        let jump = self.scroll_to_line.take();
        let gutter = self.gutter;
        let query = self.find.trim().to_owned();
        let font = TextStyle::Monospace.resolve(ui.style());
        let palette = theme::current(ui.ctx());
        let text_color = ui.visuals().text_color();
        let gutter_color = ui.visuals().weak_text_color();
        let highlight = ui.visuals().selection.bg_fill;
        let accent_fill = palette.accent;
        let accent_ink = palette.on_accent;
        let current_row = self.matches.get(self.match_cursor).copied();
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
                if Some(row) == current_row && !query.is_empty() {
                    append_highlighted_with(
                        &mut job,
                        line,
                        &query,
                        &font,
                        text_color,
                        accent_ink,
                        accent_fill,
                    );
                } else {
                    append_highlighted(&mut job, line, &query, &font, text_color, highlight);
                }
                // `Ui::label` overrides `job.wrap` with the Ui's own wrap mode, so
                // the row must be told to extend: otherwise a long line wraps,
                // grows past `row_height` and breaks the virtualised scroll.
                ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
            }
        });
    }

    /// Recompute which lines contain the find query.
    pub(crate) fn recompute_matches(&mut self) {
        let needle = self.find.trim().to_owned();
        let mut found = Vec::new();
        if !needle.is_empty()
            && let Some(body) = &self.body
        {
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
        self.matches = found;
        self.match_cursor = 0;
    }
}

/// A clickable chip showing the open capture.
///
/// Turning the file name into the way to open another capture keeps the action
/// beside its context; the hover tooltip and the pointer cursor cover the
/// discoverability a plain label would lose.
fn file_chip(
    ui: &mut egui::Ui,
    palette: &Palette,
    name: &str,
    path: &Path,
    hint: &str,
) -> egui::Response {
    let font = FontId::proportional(12.5);
    let text_width = ui.ctx().fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(name.to_owned(), font.clone(), palette.text)
            .size()
            .x
    });
    let max_width = (ui.available_width() - 40.0).max(90.0);
    let width = (text_width + 34.0).clamp(90.0, max_width);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 26.0), egui::Sense::click());
    let hovered = response.hovered();
    let fill = if hovered {
        palette.card_hover
    } else {
        palette.card
    };

    let painter = ui.painter();
    painter.rect_filled(rect, 8.0, fill);
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0, palette.card_hover),
        egui::StrokeKind::Inside,
    );
    draw_file_glyph(
        painter,
        egui::pos2(rect.left() + 14.0, rect.center().y),
        palette.accent,
    );
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 26.0, rect.top()),
        egui::pos2(rect.right() - 8.0, rect.bottom()),
    );
    painter.with_clip_rect(text_rect).text(
        text_rect.left_center(),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        palette.text,
    );

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(format!("{hint}\n{}", path.display()))
}

/// A small page glyph with a folded corner, for the file chip.
fn draw_file_glyph(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    let page = egui::Rect::from_center_size(center, egui::vec2(11.0, 14.0));
    painter.rect_stroke(
        page,
        2.0,
        egui::Stroke::new(1.4, color),
        egui::StrokeKind::Inside,
    );
    let fold = 4.5;
    let corner = page.right_top();
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(corner.x - fold, corner.y),
            egui::pos2(corner.x, corner.y + fold),
            corner,
        ],
        Color32::TRANSPARENT,
        egui::Stroke::new(1.2, color),
    ));
    painter.line_segment(
        [
            egui::pos2(page.left() + 3.0, page.center().y),
            egui::pos2(page.right() - 3.0, page.center().y),
        ],
        egui::Stroke::new(1.0, color),
    );
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
                    .id(egui::Id::new("module-filter"))
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
pub(crate) fn line_starts(text: &str) -> Vec<usize> {
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
pub(crate) fn max_line_length(text: &str, starts: &[usize]) -> usize {
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
    append_highlighted_with(job, line, query, font, color, color, highlight);
}

/// Append one line, marking matches with an explicit foreground/background.
///
/// The current match uses the accent background with on-accent text; the rest
/// use the selection background so the two states never rely on hue alone.
fn append_highlighted_with(
    job: &mut egui::text::LayoutJob,
    line: &str,
    query: &str,
    font: &FontId,
    color: Color32,
    match_color: Color32,
    match_bg: Color32,
) {
    let base = egui::TextFormat::simple(font.clone(), color);
    if query.is_empty() {
        job.append(line, 0.0, base);
        return;
    }
    let marked = egui::TextFormat {
        font_id: font.clone(),
        color: match_color,
        background: match_bg,
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

/// Truncate a module title to roughly 48 characters with an ellipsis.
///
/// The full title stays available as a tooltip, so nothing is lost.
fn ellipsize_title(title: &str) -> String {
    const LIMIT: usize = 48;
    if title.chars().count() > LIMIT {
        let mut short: String = title.chars().take(LIMIT).collect();
        short.push('…');
        short
    } else {
        title.to_owned()
    }
}

/// Whether a file-name stem is reserved on Windows (`CON`, `PRN`, …).
fn is_windows_reserved(stem: &str) -> bool {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    RESERVED.contains(&stem.to_ascii_uppercase().as_str())
}

/// Turn a module label into a safe file name fragment.
pub(crate) fn sanitize(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let capped: String = cleaned.chars().take(64).collect();
    let trimmed = capped
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_matches(|character: char| character == '_' || character.is_whitespace());
    if trimmed.is_empty() {
        return "module".to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_windows_reserved(stem) {
        return "module".to_owned();
    }
    trimmed.to_owned()
}

/// Convert a collection length into the unsigned type used by messages.
pub(crate) fn units(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// Clamp an unsigned quantity into the signed number Fluent expects.
pub(crate) fn to_number(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn human_size(bytes: usize) -> String {
    human_size_u64(units(bytes))
}

pub(crate) fn human_size_u64(bytes: u64) -> String {
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

    #[test]
    fn sanitize_strips_controls_caps_and_trailing_dots() {
        assert_eq!(sanitize("a\x00b\x1Fc"), "abc");
        assert_eq!(sanitize("name...   "), "name");
        assert_eq!(sanitize("  ___  "), "module");
        let long = "x".repeat(100);
        assert_eq!(sanitize(&long).chars().count(), 64);
    }

    #[test]
    fn sanitize_blocks_windows_reserved_names() {
        for reserved in [
            "CON", "con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1", "lpt9",
        ] {
            assert_eq!(sanitize(reserved), "module", "{reserved}");
        }
        assert_eq!(sanitize("CON.txt"), "module");
        assert_ne!(sanitize("console"), "module");
    }

    #[test]
    fn title_ellipsis_keeps_short_names_and_truncates_long_ones() {
        assert_eq!(ellipsize_title("short"), "short");
        let long = "a".repeat(60);
        let short = ellipsize_title(&long);
        assert!(short.ends_with('…'));
        assert_eq!(short.chars().count(), 49);
    }
}
