//! Welcome, home and opening animation.
//!
//! The welcome screen explains the product and offers a single Start action.
//! The home screen then offers the one way to get a capture: drop a `supout.rif`
//! on the window. The scanning view draws a folder of pages fanning open under a
//! magnifying glass, driven by the real read progress reported by the worker.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use eframe::egui::{
    self, Align2, Button, Color32, FontId, Painter, Pos2, Rect, RichText, Sense, Shape, Stroke,
    Vec2, pos2, vec2,
};

use crate::i18n::I18n;
use crate::theme::Palette;

/// Content of the scanning view for one frame.
pub struct ScanView<'a> {
    /// Elapsed seconds since the read started.
    pub elapsed: f32,
    /// Fraction read so far, when the total size is known.
    pub progress: Option<f32>,
    /// Main phase line.
    pub phase: &'a str,
    /// Secondary detail line.
    pub detail: &'a str,
}

/// Draw the welcome screen. Returns `true` when Start was pressed.
///
/// Draw the welcome screen. Returns `true` when Start was pressed.
///
/// The block is measured before it is drawn so it ends up on the true vertical
/// centre of the panel instead of being placed by a rough estimate.
pub fn welcome(ui: &mut egui::Ui, i18n: &I18n, palette: &Palette) -> bool {
    const GAP_AFTER_TITLE: f32 = 26.0;
    const GAP_AFTER_ART: f32 = 24.0;
    const GAP_BEFORE_BUTTON: f32 = 30.0;
    const BUTTON_HEIGHT: f32 = 46.0;
    const ART_HEIGHT: f32 = 104.0;

    let art = Vec2::new(ui.available_width().min(430.0), ART_HEIGHT);
    let title_font = FontId::proportional(40.0);
    let body_font = FontId::proportional(14.5);
    let body_width = ui.available_width().min(470.0);

    let title = i18n.text("app-title");
    let blurb = i18n.text("welcome-blurb");

    let title_height = ui.ctx().fonts_mut(|fonts| fonts.row_height(&title_font));
    let blurb_height = ui.ctx().fonts_mut(|fonts| {
        fonts
            .layout(blurb.clone(), body_font.clone(), palette.muted, body_width)
            .size()
            .y
    });

    let block = title_height
        + GAP_AFTER_TITLE
        + ART_HEIGHT
        + GAP_AFTER_ART
        + blurb_height
        + GAP_BEFORE_BUTTON
        + BUTTON_HEIGHT;
    ui.add_space(((ui.available_height() - block) * 0.5).max(16.0));

    let mut started = false;
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(title)
                .font(title_font)
                .strong()
                .color(palette.text),
        );
        ui.add_space(GAP_AFTER_TITLE);

        let (rect, _) = ui.allocate_exact_size(art, Sense::hover());
        draw_flow(ui.painter(), rect, palette);
        ui.add_space(GAP_AFTER_ART);

        ui.add_sized(
            [body_width, blurb_height],
            egui::Label::new(RichText::new(blurb).font(body_font).color(palette.muted))
                .wrap()
                .halign(egui::Align::Center),
        );
        ui.add_space(GAP_BEFORE_BUTTON);

        let start = ui.add(
            Button::new(
                RichText::new(i18n.text("welcome-start"))
                    .size(16.0)
                    .strong()
                    .color(palette.on_accent),
            )
            .fill(palette.accent)
            .corner_radius(12.0)
            .min_size(Vec2::new(190.0, BUTTON_HEIGHT)),
        );
        if start.clicked() {
            started = true;
        }
    });

    started
}

/// Router on the left, capture in the middle and technician on the right, joined
/// by a PCB-style trace with a USB connector: the whole point of the product.
fn draw_flow(painter: &Painter, rect: Rect, palette: &Palette) {
    let base_y = rect.bottom() - 9.0;
    let icon_y = rect.top() + 40.0;
    let left = rect.left() + 40.0;
    let middle = rect.center().x;
    let right = rect.right() - 40.0;

    let trace = Stroke::new(2.0, palette.accent_dim);
    painter.line_segment([pos2(left, base_y), pos2(right, base_y)], trace);

    for x in [
        left,
        lerp(left, middle, 0.5),
        middle,
        lerp(middle, right, 0.5),
        right,
    ] {
        painter.circle_filled(pos2(x, base_y), 3.4, palette.accent_dim);
        painter.circle_filled(pos2(x, base_y), 1.3, palette.window);
    }

    for x in [left, middle, right] {
        painter.line_segment([pos2(x, base_y), pos2(x, icon_y + 18.0)], trace);
    }

    draw_usb(painter, pos2(lerp(left, middle, 0.5), base_y), palette);
    draw_usb(painter, pos2(lerp(middle, right, 0.5), base_y), palette);

    draw_router(painter, pos2(left, icon_y), palette);
    draw_document(painter, pos2(middle, icon_y), palette);
    draw_technician(painter, pos2(right, icon_y), palette);
}

/// A small RouterOS-style access point: body, two antennas and status LEDs.
fn draw_router(painter: &Painter, center: Pos2, palette: &Palette) {
    let ink = palette.accent;
    let body = Rect::from_center_size(center + vec2(0.0, 7.0), vec2(50.0, 24.0));
    for dx in [-13.0_f32, 13.0] {
        let x = center.x + dx;
        painter.line_segment(
            [pos2(x, body.top()), pos2(x, body.top() - 15.0)],
            Stroke::new(2.4, ink),
        );
        painter.circle_filled(pos2(x, body.top() - 16.5), 2.6, ink);
    }
    painter.rect_filled(body, 6.0, ink);
    for index in 0..3 {
        painter.circle_filled(
            pos2(body.left() + 12.0 + index as f32 * 7.0, body.bottom() - 6.0),
            1.7,
            palette.on_accent,
        );
    }
}

/// The capture itself: a page with a folded corner and a few text lines.
fn draw_document(painter: &Painter, center: Pos2, palette: &Palette) {
    let page = Rect::from_center_size(center + vec2(0.0, 3.0), vec2(40.0, 50.0));
    painter.rect_filled(page, 5.0, palette.paper);
    painter.rect_stroke(
        page,
        5.0,
        Stroke::new(1.4, palette.accent),
        egui::StrokeKind::Inside,
    );

    let fold = 12.0;
    let top_right = page.right_top();
    painter.add(Shape::convex_polygon(
        vec![
            pos2(top_right.x - fold, top_right.y),
            pos2(top_right.x, top_right.y + fold),
            top_right,
        ],
        palette.paper_dim,
        Stroke::NONE,
    ));

    painter.rect_filled(
        Rect::from_min_size(pos2(page.left() + 7.0, page.top() + 7.0), vec2(11.0, 2.6)),
        1.2,
        palette.accent,
    );
    for index in 0..4 {
        let y = page.top() + 17.0 + index as f32 * 8.0;
        let width = if index == 3 { 15.0 } else { 26.0 };
        painter.rect_filled(
            Rect::from_min_size(pos2(page.left() + 7.0, y), vec2(width, 2.6)),
            1.2,
            palette.paper_line,
        );
    }
}

/// The technician: a person wearing a headset.
fn draw_technician(painter: &Painter, center: Pos2, palette: &Palette) {
    let ink = palette.accent;
    let head = center - vec2(0.0, 13.0);
    let body = Rect::from_min_size(pos2(center.x - 17.0, center.y + 1.0), vec2(34.0, 22.0));
    painter.rect_filled(body, 11.0, ink);
    painter.circle_filled(head, 8.0, ink);

    // Headband over the head, drawn as a short polyline.
    let radius = 9.2;
    let mut previous = pos2(head.x - radius, head.y);
    for index in 1..=8 {
        let angle = std::f32::consts::PI * (index as f32 / 8.0);
        let point = pos2(head.x - radius * angle.cos(), head.y - radius * angle.sin());
        painter.line_segment([previous, point], Stroke::new(2.0, ink));
        previous = point;
    }
    // Microphone boom.
    painter.line_segment(
        [
            pos2(head.x - radius, head.y + 1.0),
            pos2(head.x - radius - 2.5, head.y + 9.0),
        ],
        Stroke::new(1.6, ink),
    );
    painter.circle_filled(pos2(head.x - radius - 3.0, head.y + 10.0), 1.9, ink);
}

/// A USB plug resting on the trace.
fn draw_usb(painter: &Painter, center: Pos2, palette: &Palette) {
    let ink = palette.accent;
    let shell = Rect::from_center_size(center, vec2(18.0, 10.0));
    painter.rect_filled(shell, 2.5, ink);
    painter.add(Shape::convex_polygon(
        vec![
            pos2(shell.right(), shell.top() + 2.0),
            pos2(shell.right() + 5.0, center.y),
            pos2(shell.right(), shell.bottom() - 2.0),
        ],
        ink,
        Stroke::NONE,
    ));
    for index in 0..3 {
        let y = shell.top() + 3.0 + index as f32 * 2.0;
        painter.line_segment(
            [pos2(shell.left() + 3.0, y), pos2(shell.right() - 2.0, y)],
            Stroke::new(0.8, palette.on_accent),
        );
    }
}

/// Draw the home screen. Returns `true` when the user asked to choose a file.
pub fn home(ui: &mut egui::Ui, i18n: &I18n, palette: &Palette, file_hovered: bool) -> bool {
    let zone = Vec2::new(560.0, 240.0);
    ui.add_space(((ui.available_height() - zone.y) * 0.5).max(16.0));

    let mut chosen = false;
    ui.vertical_centered(|ui| {
        let (rect, response) = ui.allocate_exact_size(zone, Sense::click());
        let hovered = response.hovered() || file_hovered;
        draw_drop_zone(ui, rect, hovered, palette, i18n);
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if response.clicked() {
            chosen = true;
        }
    });

    chosen
}

/// Draw the drop target: dashed outline, folder glyph and instructions.
fn draw_drop_zone(ui: &mut egui::Ui, rect: Rect, hovered: bool, palette: &Palette, i18n: &I18n) {
    let painter = ui.painter();
    let fill = if hovered {
        palette.card_hover
    } else {
        palette.card
    };
    painter.rect_filled(rect, 20.0, fill);

    let edge = if hovered {
        palette.accent
    } else {
        palette.accent_dim
    };
    let path = vec![
        rect.left_top() + vec2(3.0, 3.0),
        rect.right_top() + vec2(-3.0, 3.0),
        rect.right_bottom() + vec2(-3.0, -3.0),
        rect.left_bottom() + vec2(3.0, -3.0),
        rect.left_top() + vec2(3.0, 3.0),
    ];
    painter.extend(Shape::dashed_line(&path, Stroke::new(2.0, edge), 10.0, 8.0));

    draw_folder(painter, rect.center() - vec2(0.0, 40.0), hovered, palette);

    let title_id = if hovered {
        "drop-title-hover"
    } else {
        "drop-title"
    };
    ui.painter().text(
        rect.center() + vec2(0.0, 40.0),
        Align2::CENTER_CENTER,
        i18n.text(title_id),
        FontId::proportional(19.0),
        palette.text,
    );
    ui.painter().text(
        rect.center() + vec2(0.0, 70.0),
        Align2::CENTER_CENTER,
        i18n.text("drop-hint"),
        FontId::proportional(13.0),
        palette.muted,
    );
}

/// A clean open-folder glyph drawn from rounded rectangles.
fn draw_folder(painter: &Painter, center: Pos2, hovered: bool, palette: &Palette) {
    let width = 86.0;
    let height = 62.0;
    let rect = Rect::from_center_size(center, vec2(width, height));
    let back = if hovered {
        palette.accent
    } else {
        palette.accent_dim
    };

    // Back panel with the tab on top-left.
    let tab = Rect::from_min_size(pos2(rect.left(), rect.top()), vec2(32.0, 13.0));
    painter.rect_filled(tab, 4.0, back);
    let body = Rect::from_min_max(
        pos2(rect.left(), rect.top() + 9.0),
        pos2(rect.right(), rect.bottom() - 12.0),
    );
    painter.rect_filled(body, 7.0, back);

    // Front flap, slightly narrower and lower, in the accent colour.
    let front = Rect::from_min_max(
        pos2(rect.left() + 3.0, rect.top() + 26.0),
        pos2(rect.right() - 3.0, rect.bottom()),
    );
    painter.rect_filled(front, 7.0, palette.accent);
    painter.rect_stroke(front, 7.0, Stroke::new(1.0, back), egui::StrokeKind::Inside);
}

/// Draw the "reading the capture" animation, centred in the window.
pub fn scanning(ui: &mut egui::Ui, view: &ScanView<'_>, palette: &Palette) {
    const BLOCK_HEIGHT: f32 = 400.0;
    let top_gap = ((ui.available_height() - BLOCK_HEIGHT) * 0.5).max(16.0);
    ui.add_space(top_gap);

    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(460.0, 290.0), Sense::hover());
        draw_scanner(ui.painter(), rect, view.elapsed, palette);

        ui.add_space(18.0);
        ui.label(
            RichText::new(view.phase)
                .size(19.0)
                .strong()
                .color(palette.text),
        );
        ui.add_space(4.0);
        ui.label(RichText::new(view.detail).size(13.0).color(palette.muted));
        ui.add_space(18.0);

        let (bar_rect, _) = ui.allocate_exact_size(Vec2::new(420.0, 6.0), Sense::hover());
        draw_progress(ui.painter(), bar_rect, view.progress, view.elapsed, palette);
    });
}

/// Draw the page stack, the sweep beam and the magnifying glass.
fn draw_scanner(painter: &Painter, rect: Rect, elapsed: f32, palette: &Palette) {
    let center = rect.center();
    let glow = palette.accent.linear_multiply(0.08);
    painter.circle_filled(center, 170.0, glow);
    painter.circle_filled(center, 110.0, glow);

    let base = Rect::from_center_size(center, vec2(206.0, 244.0));
    let open = ease_out_cubic((elapsed / 0.9).clamp(0.0, 1.0));

    // Pages fanning open on both sides.
    let left = rotate_polygon(base, base.left_bottom(), -0.22 * open);
    painter.add(Shape::convex_polygon(left, palette.paper_dim, Stroke::NONE));
    let right = rotate_polygon(base, base.right_bottom(), 0.22 * open);
    painter.add(Shape::convex_polygon(
        right,
        palette.paper_dim,
        Stroke::NONE,
    ));

    // Front page with a folded corner.
    painter.rect_filled(base, 11.0, palette.paper);
    let fold = 28.0;
    let tr = base.right_top();
    painter.add(Shape::convex_polygon(
        vec![pos2(tr.x - fold, tr.y), pos2(tr.x, tr.y + fold), tr],
        palette.paper_dim,
        Stroke::NONE,
    ));

    // Sweep position: loops across the page, wobbling vertically.
    let sweep = (elapsed * 0.55).fract();
    let beam_x = lerp(base.left() + 18.0, base.right() - 18.0, sweep);

    // Text lines, revealed as the beam passes.
    for index in 0..9usize {
        let y = base.top() + 42.0 + index as f32 * 19.0;
        let width = match index % 4 {
            0 => 142.0,
            1 => 112.0,
            2 => 156.0,
            _ => 92.0,
        };
        let line = Rect::from_min_size(pos2(base.left() + 24.0, y), vec2(width, 6.0));
        let color = if line.right() < beam_x {
            palette.accent_dim
        } else {
            palette.paper_line
        };
        painter.rect_filled(line, 3.0, color);
    }

    // Sweep beam glow.
    for (offset, alpha) in [(0.0_f32, 46u8), (5.0, 28), (10.0, 14)] {
        let color = Color32::from_rgba_unmultiplied(
            palette.accent.r(),
            palette.accent.g(),
            palette.accent.b(),
            alpha,
        );
        let band = Rect::from_min_max(
            pos2(beam_x + offset - 1.5, base.top() + 22.0),
            pos2(beam_x + offset + 1.5, base.bottom() - 22.0),
        );
        painter.rect_filled(band, 1.5, color);
    }

    // Magnifying glass following the beam.
    let lens = pos2(beam_x, base.center().y + (elapsed * 2.2).sin() * 52.0);
    painter.circle_filled(lens, 42.0, palette.accent.linear_multiply(0.12));
    painter.circle_stroke(lens, 42.0, Stroke::new(3.5, palette.accent));
    let pulse = 48.0 + (elapsed * 3.0).sin() * 5.0;
    painter.circle_stroke(
        lens,
        pulse,
        Stroke::new(1.0, palette.accent.linear_multiply(0.3)),
    );

    for index in 0..3 {
        let y = lens.y - 11.0 + index as f32 * 11.0;
        let half = 20.0 - index as f32 * 4.0;
        painter.line_segment(
            [pos2(lens.x - half, y), pos2(lens.x + half, y)],
            Stroke::new(4.0, palette.accent.linear_multiply(0.7)),
        );
    }

    let direction = vec2(0.72, 0.69);
    painter.line_segment(
        [lens + direction * 40.0, lens + direction * 70.0],
        Stroke::new(9.0, palette.accent),
    );
    painter.line_segment(
        [lens + direction * 40.0, lens + direction * 70.0],
        Stroke::new(4.0, palette.paper),
    );
}

/// Draw the progress bar, determinate when a fraction is known, looping otherwise.
fn draw_progress(
    painter: &Painter,
    rect: Rect,
    progress: Option<f32>,
    elapsed: f32,
    palette: &Palette,
) {
    painter.rect_filled(rect, 3.0, palette.card);
    let fraction = progress.unwrap_or_else(|| {
        let t = (elapsed * 0.9).fract();
        (t * 1.4 - 0.2).clamp(0.0, 1.0)
    });
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction <= 0.0 {
        return;
    }
    let filled = Rect::from_min_size(
        rect.min,
        vec2((rect.width() * fraction).max(4.0), rect.height()),
    );
    painter.rect_filled(filled, 3.0, palette.accent);
}

/// Rotate a rectangle's corners around `pivot`.
fn rotate_polygon(rect: Rect, pivot: Pos2, angle: f32) -> Vec<Pos2> {
    let (sin, cos) = angle.sin_cos();
    let rotate = |point: Pos2| {
        let delta = point - pivot;
        pos2(
            pivot.x + delta.x * cos - delta.y * sin,
            pivot.y + delta.x * sin + delta.y * cos,
        )
    };
    vec![
        rotate(rect.left_top()),
        rotate(rect.right_top()),
        rotate(rect.right_bottom()),
        rotate(rect.left_bottom()),
    ]
}

fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}
