//! About tab: version, copyright, repository link and credits.
//!
//! Split out of `settings.rs`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use eframe::egui::{self, RichText};
use fluent_bundle::FluentArgs;

use crate::app::Viewer;
use crate::build_info;
use crate::theme::Palette;
use crate::update;

impl Viewer {
    /// Version, copyright, repository link and credits.
    pub(super) fn about_tab(&mut self, ui: &mut egui::Ui, palette: &Palette, ctx: &egui::Context) {
        let mut version_args = FluentArgs::new();
        version_args.set("version", build_info::VERSION.to_owned());
        let version_label = self.i18n.render("about-version", &version_args);
        let copyright = self.i18n.text("about-copyright");
        let license = self.i18n.text("about-license");
        let license_link = self.i18n.text("about-license-link");
        let repo_label = self.i18n.text("about-repository");
        let credits_heading = self.i18n.text("about-credits-heading");
        let credits_body = self.i18n.text("about-credits-body");
        let trademark = self.i18n.text("about-trademark");

        ui.add_space(10.0);
        // The product name is a constant, not a string to look up per frame.
        ui.label(
            RichText::new(crate::parser::PRODUCT_NAME)
                .size(16.0)
                .strong()
                .color(palette.text),
        );
        ui.label(RichText::new(&version_label).color(palette.muted));
        ui.label(RichText::new(copyright).color(palette.muted));
        ui.label(RichText::new(license).color(palette.muted));

        ui.add_space(12.0);
        let texture = self.github_texture(ctx);
        let image = egui::Image::from_texture(egui::load::SizedTexture::from_handle(&texture))
            .fit_to_exact_size(egui::vec2(16.0, 16.0))
            .tint(palette.accent);
        ui.horizontal(|ui| {
            ui.add(image);
            if ui
                .link(RichText::new(&repo_label).color(palette.accent))
                .clicked()
            {
                ctx.open_url(egui::OpenUrl::new_tab(update::repository_url()));
            }
        });
        if ui.link(license_link).clicked() {
            ctx.open_url(egui::OpenUrl::new_tab(update::license_url()));
        }

        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label(RichText::new(&credits_heading).strong().color(palette.text));
        ui.label(RichText::new(credits_body).size(12.0).color(palette.muted));

        ui.add_space(14.0);
        ui.label(RichText::new(trademark).size(11.5).color(palette.muted));
    }

    /// Upload the bundled GitHub mark once, returning the cached texture.
    ///
    /// The asset is GitHub's official mark, normalised to a white silhouette
    /// (not redrawn) so it can be tinted to the current theme's text colour.
    fn github_texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        if let Some(texture) = &self.github_texture {
            return texture.clone();
        }
        let texture = load_github_texture(ctx);
        self.github_texture = Some(texture.clone());
        texture
    }
}

/// The official GitHub mark, normalised to a white-on-transparent silhouette
/// so it can be tinted to the theme's text colour. The geometry is GitHub's,
/// unmodified; the asset is used only to link to the project repository.
pub(super) const GITHUB_MARK_PNG: &[u8] = include_bytes!("../../../assets/icon/github.png");

/// Decode and upload the bundled GitHub mark as a texture.
fn load_github_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let data = eframe::icon_data::from_png_bytes(GITHUB_MARK_PNG).unwrap_or_default();
    let image = egui::ColorImage::from_rgba_unmultiplied(
        [data.width as usize, data.height as usize],
        &data.rgba,
    );
    ctx.load_texture("github-mark", image, egui::TextureOptions::LINEAR)
}
