//! Small vector icons, drawn with the painter.
//!
//! The interface bundles no icon font, so every glyph is drawn from a few
//! strokes. That keeps them crisp at any scale, consistent in both themes and
//! independent of whatever symbols a font happens to ship.

use eframe::egui::{self, Color32, Painter, Pos2, Sense, Stroke, Vec2, pos2};

use crate::theme::Palette;

/// Which glyph an icon button draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// Magnifying glass.
    Magnifier,
    /// Chevron pointing up.
    ChevronUp,
    /// Chevron pointing down.
    ChevronDown,
    /// Cross used to clear or close.
    Close,
    /// A rectangle with a highlighted side column, for the module rail.
    PanelLeft,
    /// A cog wheel, for the settings entry point.
    Gear,
}

/// A small square icon button with an optional selected state.
///
/// Returns `true` on the frame it is clicked. Disabled buttons still draw, in a
/// muted colour, and never react.
pub fn icon_button(
    ui: &mut egui::Ui,
    palette: &Palette,
    glyph: Glyph,
    selected: bool,
    enabled: bool,
    hint: &str,
) -> bool {
    icon_button_ext(ui, palette, glyph, selected, enabled, hint, false)
}

/// [`icon_button`] with an optional notification dot in the top-right corner.
///
/// The dot marks state that lives behind the button (used by the settings gear
/// when an update is available) without adding a permanent label.
pub fn icon_button_ext(
    ui: &mut egui::Ui,
    palette: &Palette,
    glyph: Glyph,
    selected: bool,
    enabled: bool,
    hint: &str,
    badge: bool,
) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::splat(26.0),
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let hovered = enabled && response.hovered();
    let background = if selected {
        palette.accent.linear_multiply(0.20)
    } else if hovered {
        palette.card_hover
    } else {
        Color32::TRANSPARENT
    };
    let color = if !enabled {
        palette.muted.linear_multiply(0.45)
    } else if selected {
        palette.accent
    } else if hovered {
        palette.text
    } else {
        palette.muted
    };

    let painter = ui.painter();
    if background != Color32::TRANSPARENT {
        painter.rect_filled(rect, 7.0, background);
    }
    draw(painter, rect.center(), glyph, color, selected);

    if badge && enabled {
        let dot = rect.right_top() + Vec2::new(-4.0, 4.0);
        // A panel-coloured ring separates the dot from the glyph underneath.
        painter.circle_filled(dot, 4.5, palette.panel);
        painter.circle_filled(dot, 3.0, palette.accent);
    }

    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.on_hover_text(hint).clicked() && enabled
}

/// Render one glyph centred on `center`.
fn draw(painter: &Painter, center: Pos2, glyph: Glyph, color: Color32, selected: bool) {
    let stroke = Stroke::new(1.6, color);
    match glyph {
        Glyph::Magnifier => {
            let lens = center - Vec2::new(1.2, 1.2);
            let radius = 5.6;
            painter.circle_stroke(lens, radius, Stroke::new(1.7, color));
            let diagonal = Vec2::new(0.707, 0.707);
            let tip = lens + diagonal * (radius + 4.6);
            painter.line_segment(
                [lens + diagonal * (radius - 0.4), tip],
                Stroke::new(2.1, color),
            );
            // Round off the handle tip: epaint draws butt caps by default.
            painter.circle_filled(tip, 1.05, color);
        }
        Glyph::ChevronUp => {
            let point = |x: f32, y: f32| center + Vec2::new(x, y);
            painter.line_segment([point(-4.2, 1.6), point(0.0, -2.6)], stroke);
            painter.line_segment([point(0.0, -2.6), point(4.2, 1.6)], stroke);
        }
        Glyph::ChevronDown => {
            let point = |x: f32, y: f32| center + Vec2::new(x, y);
            painter.line_segment([point(-4.2, -1.6), point(0.0, 2.6)], stroke);
            painter.line_segment([point(0.0, 2.6), point(4.2, -1.6)], stroke);
        }
        Glyph::Close => {
            let point = |x: f32, y: f32| center + Vec2::new(x, y);
            painter.line_segment([point(-3.6, -3.6), point(3.6, 3.6)], stroke);
            painter.line_segment([point(-3.6, 3.6), point(3.6, -3.6)], stroke);
        }
        Glyph::PanelLeft => {
            let frame = egui::Rect::from_center_size(center, Vec2::new(16.0, 12.0));
            if selected {
                painter.rect_filled(
                    egui::Rect::from_min_max(frame.min, pos2(frame.left() + 5.5, frame.bottom())),
                    2.0,
                    color,
                );
            }
            painter.rect_stroke(frame, 2.5, stroke, egui::StrokeKind::Inside);
            painter.line_segment(
                [
                    pos2(frame.left() + 5.5, frame.top()),
                    pos2(frame.left() + 5.5, frame.bottom()),
                ],
                stroke,
            );
        }
        Glyph::Gear => {
            let radius = 5.4;
            painter.circle_stroke(center, radius, Stroke::new(1.5, color));
            painter.circle_stroke(center, 2.0, Stroke::new(1.5, color));
            for index in 0..8u8 {
                let angle = std::f32::consts::TAU * (f32::from(index) / 8.0);
                let (sin, cos) = angle.sin_cos();
                let direction = Vec2::new(cos, sin);
                painter.line_segment(
                    [
                        center + direction * (radius - 0.4),
                        center + direction * (radius + 2.1),
                    ],
                    Stroke::new(1.5, color),
                );
            }
        }
    }
}
