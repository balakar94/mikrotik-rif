//! Palettes, typography and OS integration.
//!
//! The app follows the operating system: `ThemePreference::System` makes egui
//! pick light or dark visuals, and the custom-painted screens read the matching
//! [`Palette`] so hand-drawn elements change with the system too.

use std::collections::BTreeMap;
use std::sync::Arc;

use eframe::egui::{
    self, Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Mesh,
    Rect, Shape, Stroke, TextStyle, Theme, ThemePreference,
};

const INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const NOTO_SANS_SC: &[u8] = include_bytes!("../assets/fonts/NotoSansSC-Regular.otf");

/// Family name of the UI font.
pub const PROPORTIONAL: &str = "inter";
/// Family name of the code font.
pub const MONOSPACE: &str = "jetbrains-mono";
/// Family name of the CJK fallback font.
pub const CJK: &str = "noto-sans-sc";

/// Colours for one theme.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Window background.
    pub window: Color32,
    /// Bars and rails.
    pub panel: Color32,
    /// Top of the background gradient behind the first screens.
    pub bg_top: Color32,
    /// Bottom of the background gradient behind the first screens.
    pub bg_bottom: Color32,
    /// Cards and controls.
    pub card: Color32,
    /// Cards under the pointer.
    pub card_hover: Color32,
    /// Primary accent.
    pub accent: Color32,
    /// Resting accent outline.
    pub accent_dim: Color32,
    /// Text drawn on top of the accent.
    pub on_accent: Color32,
    /// Primary text.
    pub text: Color32,
    /// Secondary text.
    pub muted: Color32,
    /// Error text.
    pub danger: Color32,
    /// Paper used by the scanning animation.
    pub paper: Color32,
    /// Paper edges and shadows in the animation.
    pub paper_dim: Color32,
    /// Text lines drawn on the animated paper.
    pub paper_line: Color32,
}

const DARK: Palette = Palette {
    window: Color32::from_rgb(0x0B, 0x0F, 0x14),
    panel: Color32::from_rgb(0x0F, 0x16, 0x20),
    bg_top: Color32::from_rgb(0x0D, 0x17, 0x27),
    bg_bottom: Color32::from_rgb(0x08, 0x0B, 0x10),
    card: Color32::from_rgb(0x15, 0x20, 0x2A),
    card_hover: Color32::from_rgb(0x1E, 0x2C, 0x39),
    accent: Color32::from_rgb(0x3B, 0xBD, 0xF8),
    accent_dim: Color32::from_rgb(0x21, 0x60, 0x80),
    on_accent: Color32::from_rgb(0x05, 0x10, 0x18),
    text: Color32::from_rgb(0xE8, 0xEE, 0xF4),
    muted: Color32::from_rgb(0x95, 0xA3, 0xB2),
    danger: Color32::from_rgb(0xF8, 0x71, 0x71),
    paper: Color32::from_rgb(0x1A, 0x24, 0x30),
    paper_dim: Color32::from_rgb(0x2A, 0x36, 0x44),
    paper_line: Color32::from_rgb(0x3A, 0x4A, 0x5C),
};

const LIGHT: Palette = Palette {
    window: Color32::from_rgb(0xF4, 0xF7, 0xFB),
    panel: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    bg_top: Color32::from_rgb(0xE3, 0xED, 0xF9),
    bg_bottom: Color32::from_rgb(0xF7, 0xFA, 0xFD),
    card: Color32::from_rgb(0xE9, 0xF0, 0xF7),
    card_hover: Color32::from_rgb(0xDB, 0xE6, 0xF1),
    accent: Color32::from_rgb(0x0B, 0x6F, 0xB0),
    accent_dim: Color32::from_rgb(0x6C, 0xA4, 0xC8),
    on_accent: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    text: Color32::from_rgb(0x10, 0x15, 0x1B),
    muted: Color32::from_rgb(0x49, 0x54, 0x62),
    danger: Color32::from_rgb(0xB4, 0x23, 0x18),
    paper: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    paper_dim: Color32::from_rgb(0xC6, 0xD2, 0xE0),
    paper_line: Color32::from_rgb(0xAE, 0xBB, 0xCC),
};

/// Palette for a resolved theme.
#[must_use]
pub fn of(dark: bool) -> Palette {
    if dark { DARK } else { LIGHT }
}

/// Palette matching the theme currently resolved for a context.
#[must_use]
pub fn current(ctx: &Context) -> Palette {
    of(ctx.theme() == Theme::Dark)
}

/// Paint the soft vertical gradient that backs the first screens.
///
/// A mesh with per-vertex colours keeps this a single draw call, so filling the
/// whole panel every frame costs nothing noticeable.
pub fn paint_background(painter: &egui::Painter, rect: Rect, palette: &Palette) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), palette.bg_top);
    mesh.colored_vertex(rect.right_top(), palette.bg_top);
    mesh.colored_vertex(rect.right_bottom(), palette.bg_bottom);
    mesh.colored_vertex(rect.left_bottom(), palette.bg_bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(Shape::mesh(mesh));
}

/// Human-readable name of the host platform.
///
/// `std::env::consts::OS` returns lowercase identifiers such as `macos`; the
/// footer shows the conventional spelling instead.
#[must_use]
pub fn platform_name() -> &'static str {
    platform_name_for(std::env::consts::OS)
}

/// Map a `std::env::consts::OS` value to its conventional spelling.
#[must_use]
pub fn platform_name_for(os: &'static str) -> &'static str {
    match os {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        // FreeBSD is a supported Rust target, so it keeps a proper name.
        "freebsd" => "FreeBSD",
        other => other,
    }
}

/// Install fonts, follow the system theme and apply both palettes.
pub fn apply(ctx: &Context) {
    ctx.set_fonts(fonts());
    ctx.set_theme(ThemePreference::System);
    ctx.set_visuals_of(Theme::Dark, visuals(DARK, true));
    ctx.set_visuals_of(Theme::Light, visuals(LIGHT, false));

    ctx.all_styles_mut(|style| {
        style.text_styles = text_styles();
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(16.0, 9.0);
        style.spacing.interact_size.y = 26.0;
        style.visuals.window_corner_radius = CornerRadius::same(12);
        style.visuals.menu_corner_radius = CornerRadius::same(10);
        for widget in [
            &mut style.visuals.widgets.noninteractive,
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
            &mut style.visuals.widgets.open,
        ] {
            widget.corner_radius = CornerRadius::same(9);
        }
        // Visible keyboard focus in both themes: keep the per-theme accent
        // colour set in `visuals()` and widen the ring so Tab focus is
        // distinguishable from hover. `selection.stroke` is what egui uses
        // for the focus outline.
        style.visuals.selection.stroke.width = 2.0;
    });
}

/// Register the bundled fonts as the first choice of each family.
///
/// Latin and Cyrillic resolve through the primary faces; scripts they do not
/// cover (CJK, above all) fall through to the bundled Noto Sans SC.
fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        PROPORTIONAL.to_owned(),
        Arc::new(FontData::from_static(INTER)),
    );
    fonts.font_data.insert(
        MONOSPACE.to_owned(),
        Arc::new(FontData::from_static(JETBRAINS_MONO)),
    );
    fonts.font_data.insert(
        CJK.to_owned(),
        Arc::new(FontData::from_static(NOTO_SANS_SC)),
    );

    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, PROPORTIONAL.to_owned());
    proportional.insert(1, CJK.to_owned());

    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.insert(0, MONOSPACE.to_owned());
    monospace.insert(1, CJK.to_owned());

    fonts
}

/// Type scale for the whole interface.
fn text_styles() -> BTreeMap<TextStyle, FontId> {
    BTreeMap::from([
        (
            TextStyle::Small,
            FontId::new(11.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
    ])
}

/// Build the egui visuals for one palette.
fn visuals(palette: Palette, dark: bool) -> egui::Visuals {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = palette.panel;
    visuals.window_fill = palette.window;
    visuals.extreme_bg_color = palette.card;
    visuals.faint_bg_color = palette.card;
    visuals.hyperlink_color = palette.accent;
    visuals.window_stroke = Stroke::new(1.0, palette.card_hover);
    visuals.selection.bg_fill = palette
        .accent
        .linear_multiply(if dark { 0.45 } else { 0.24 });
    visuals.selection.stroke = Stroke::new(2.0, palette.accent);

    visuals.widgets.noninteractive.bg_fill = palette.panel;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.inactive.bg_fill = palette.card;
    visuals.widgets.inactive.weak_bg_fill = palette.card;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, palette.text);
    visuals.widgets.hovered.bg_fill = palette.card_hover;
    visuals.widgets.hovered.weak_bg_fill = palette.card_hover;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, palette.text);
    // Pressed state uses the full accent with on-accent text so it passes
    // WCAG AA in both themes and stays clearly distinct from the
    // `card_hover` hover/open states.
    visuals.widgets.active.bg_fill = palette.accent;
    visuals.widgets.active.weak_bg_fill = palette.accent;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, palette.on_accent);
    visuals.widgets.open.bg_fill = palette.card_hover;
    visuals.widgets.open.weak_bg_fill = palette.card_hover;
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, palette.text);
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_names_use_conventional_spelling() {
        assert_eq!(platform_name_for("macos"), "macOS");
        assert_eq!(platform_name_for("windows"), "Windows");
        assert_eq!(platform_name_for("linux"), "Linux");
        assert_eq!(platform_name_for("freebsd"), "FreeBSD");
    }

    #[test]
    fn unknown_platforms_pass_through() {
        assert_eq!(platform_name_for("dragonfly"), "dragonfly");
    }
}
