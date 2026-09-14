//! Settings screen: theme, language, updates and About.
//!
//! Shown as a centred modal (`egui::Modal`) so it can be opened from any stage
//! without unmounting the workspace behind it. The updater's own state machine
//! lives in `super::update_ui`; this module only presents it. Theme and
//! language preferences are persisted through eframe storage (see
//! [`Viewer::restore_prefs`]).

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use eframe::egui::{self, Align, Color32, Layout, RichText, ThemePreference};
use fluent_bundle::FluentArgs;

use crate::i18n;
use crate::theme::{self, Palette};

use super::{LANGUAGE_KEY, THEME_KEY, Viewer};

/// Which tab the settings modal is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    /// Theme and language.
    General,
    /// Version, build hash and the update check.
    Updates,
    /// Version, copyright, repository link and credits.
    About,
}

/// What a click in the Updates tab asked for; applied after drawing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateAction {
    /// Start a manual check.
    Check,
    /// Download the offered release.
    Install,
    /// Hand the verified installer to the operating system.
    Launch,
    /// Never offer this release again.
    Skip,
    /// Open the releases page in a browser.
    OpenPage,
}

/// Storage token for a theme preference.
pub(crate) const fn theme_storage_key(pref: ThemePreference) -> &'static str {
    match pref {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
        ThemePreference::System => "system",
    }
}

/// Parse a stored theme preference; anything unknown means "follow system".
fn theme_from_storage(value: &str) -> ThemePreference {
    match value {
        "dark" => ThemePreference::Dark,
        "light" => ThemePreference::Light,
        _ => ThemePreference::System,
    }
}

/// Draw the settings gear, with a badge when an update is available.
pub(crate) fn gear_button(ui: &mut egui::Ui, palette: &Palette, badge: bool, hint: &str) -> bool {
    crate::icons::icon_button_ext(
        ui,
        palette,
        crate::icons::Glyph::Gear,
        false,
        true,
        hint,
        badge,
    )
}

impl Viewer {
    /// Restore persisted theme and language, applying the theme immediately.
    pub(crate) fn restore_prefs(
        &mut self,
        storage: Option<&dyn eframe::Storage>,
        ctx: &egui::Context,
    ) {
        self.restore_update_prefs(storage);
        let Some(storage) = storage else {
            return;
        };
        if let Some(value) = storage.get_string(THEME_KEY) {
            self.theme_pref = theme_from_storage(&value);
            ctx.set_theme(self.theme_pref);
        }
        if let Some(value) = storage.get_string(LANGUAGE_KEY)
            && !value.is_empty()
        {
            self.i18n.set_language(&value);
            self.language = value;
        }
    }

    /// Switch the interface language; an empty tag follows the system locale.
    pub(crate) fn choose_language(&mut self, tag: &str) {
        if tag.is_empty() {
            self.language.clear();
            self.i18n = i18n::I18n::detected();
        } else {
            tag.clone_into(&mut self.language);
            self.i18n.set_language(tag);
        }
    }

    /// Tooltip for the settings gear, mentioning a pending update when there is
    /// one.
    pub(crate) fn settings_hint(&self) -> String {
        match self.update_version() {
            Some(version) if self.update_available() => {
                let mut args = FluentArgs::new();
                args.set("version", version.to_owned());
                self.i18n.render("button-settings-update", &args)
            }
            _ => self.i18n.text("button-settings"),
        }
    }

    /// Floating gear used on the screens that have no header of their own.
    pub(crate) fn settings_access(&mut self, ctx: &egui::Context) {
        let palette = theme::current(ctx);
        let hint = self.settings_hint();
        let badge = self.update_available();
        let mut clicked = false;
        egui::Area::new(egui::Id::new("settings-access"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 10.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                clicked = gear_button(ui, &palette, badge, &hint);
            });
        if clicked {
            self.settings = Some(SettingsTab::General);
        }
    }

    /// Render the settings modal for this frame.
    pub(crate) fn settings_modal(&mut self, ctx: &egui::Context) {
        let Some(mut tab) = self.settings else {
            return;
        };
        let palette = theme::current(ctx);
        let title = self.i18n.text("settings-title");
        let close_hint = self.i18n.text("settings-close");
        let tab_general = self.i18n.text("settings-tab-general");
        let tab_updates = self.i18n.text("settings-tab-updates");
        let tab_about = self.i18n.text("settings-tab-about");

        let available = ctx.content_rect();
        let width = (available.width() - 48.0).clamp(360.0, 560.0);
        // A cap, not a fixed height: the modal still shrinks to its content.
        let max_height = (available.height() - 200.0).max(200.0);
        let frame = egui::Frame::NONE
            .fill(palette.panel)
            .stroke(egui::Stroke::new(1.0, palette.card_hover))
            .corner_radius(14.0)
            .inner_margin(egui::Margin::same(16));

        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("settings-modal"))
            .backdrop_color(Color32::from_black_alpha(
                if ctx.theme() == egui::Theme::Dark {
                    150
                } else {
                    60
                },
            ))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(&title)
                            .size(16.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if crate::icons::icon_button(
                            ui,
                            &palette,
                            crate::icons::Glyph::Close,
                            false,
                            true,
                            &close_hint,
                        ) {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                let mut active_tab = None;
                ui.horizontal(|ui| {
                    let general = ui.selectable_value(&mut tab, SettingsTab::General, tab_general);
                    let updates = ui.selectable_value(&mut tab, SettingsTab::Updates, tab_updates);
                    let about = ui.selectable_value(&mut tab, SettingsTab::About, tab_about);
                    active_tab = Some(match tab {
                        SettingsTab::General => general,
                        SettingsTab::Updates => updates,
                        SettingsTab::About => about,
                    });
                });
                // Give the modal keyboard focus the first time it opens, so Tab
                // and the arrow keys reach its contents without a mouse click.
                if ctx.memory(|memory| memory.focused().is_none())
                    && let Some(response) = active_tab
                {
                    response.request_focus();
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .max_height(max_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| match tab {
                        SettingsTab::General => self.general_tab(ui, &palette, ctx),
                        SettingsTab::Updates => self.updates_tab(ui, &palette, ctx),
                        SettingsTab::About => self.about_tab(ui, &palette, ctx),
                    });
            });

        self.settings = if close || response.should_close() {
            None
        } else {
            Some(tab)
        };
    }

    /// Theme and language controls.
    fn general_tab(&mut self, ui: &mut egui::Ui, palette: &Palette, ctx: &egui::Context) {
        let theme_label = self.i18n.text("settings-theme");
        let system = self.i18n.text("theme-system");
        let light = self.i18n.text("theme-light");
        let dark = self.i18n.text("theme-dark");
        let language_label = self.i18n.text("settings-language");
        let system_language = self.i18n.text("settings-language-system");

        ui.add_space(10.0);
        ui.label(RichText::new(&theme_label).strong().color(palette.text));
        ui.add_space(4.0);
        // A local copy so a change can be applied to the context in the same
        // frame instead of one frame late.
        let mut theme = self.theme_pref;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut theme, ThemePreference::System, system);
            ui.selectable_value(&mut theme, ThemePreference::Light, light);
            ui.selectable_value(&mut theme, ThemePreference::Dark, dark);
        });
        if theme != self.theme_pref {
            self.theme_pref = theme;
            ctx.set_theme(theme);
        }

        ui.add_space(18.0);
        ui.separator();
        ui.add_space(10.0);
        ui.label(RichText::new(&language_label).strong().color(palette.text));
        ui.add_space(4.0);

        let current = if self.language.is_empty() {
            system_language.clone()
        } else {
            i18n::endonym(&self.language)
        };
        let mut selected = self.language.clone();
        egui::ComboBox::from_id_salt("settings-language")
            .selected_text(current)
            .width(220.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, String::new(), system_language.as_str());
                for tag in i18n::shipped_languages() {
                    ui.selectable_value(&mut selected, tag.to_owned(), i18n::endonym(tag));
                }
            });
        if selected != self.language {
            self.choose_language(&selected);
        }
    }
}

mod about;
mod updates;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_storage_keys_round_trip() {
        for pref in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            assert_eq!(theme_from_storage(theme_storage_key(pref)), pref);
        }
    }

    #[test]
    fn unknown_stored_theme_means_system() {
        assert_eq!(theme_from_storage("nonsense"), ThemePreference::System);
        assert_eq!(theme_from_storage(""), ThemePreference::System);
    }

    #[test]
    fn bundled_github_mark_decodes_as_a_square_rgba_image() {
        let data =
            eframe::icon_data::from_png_bytes(super::about::GITHUB_MARK_PNG).expect("valid png");
        assert_eq!(data.width, 256);
        assert_eq!(data.height, 256);
        assert_eq!(data.rgba.len(), 256 * 256 * 4);
    }
}
