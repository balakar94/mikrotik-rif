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
    }
}
