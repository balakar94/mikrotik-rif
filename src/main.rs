//! MikroTik RIF Viewer — single cross-platform desktop application.
//!
//! The whole product lives in this one package: the `parser` module reads the
//! capture format, and the UI modules (`app`, `splash`, `theme`, `worker`) render
//! it. All work happens in-process; the only network path anywhere is the
//! opt-out update check against the GitHub releases API (see `update`).

#![forbid(unsafe_code)]
// GUI application on Windows: without this the linker assumes a console
// subsystem and Windows opens a terminal window next to the app. Gated on
// release builds so `cargo run` during development keeps stdout/stderr visible.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod build_info;
mod i18n;
mod icons;
mod panic;
// The parser is an internal, self-contained library. Part of its surface is
// exercised only by tests or reserved for upcoming features, so dead-code
// analysis is relaxed for this module only.
#[allow(dead_code)]
mod parser;
mod splash;
mod theme;
mod update;
mod worker;

use eframe::egui;
use parser::PRODUCT_NAME;

fn main() -> eframe::Result {
    panic::install();

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/icon.png"))
        .unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(PRODUCT_NAME)
            // Used for the on-disk preferences folder (and Wayland's app id);
            // matches the bundle identifier in `Cargo.toml`.
            .with_app_id("io.github.balakar94.mikrotik-rif")
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([820.0, 520.0])
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        PRODUCT_NAME,
        options,
        Box::new(|cc| Ok(Box::new(app::Viewer::new(cc)))),
    )
}
