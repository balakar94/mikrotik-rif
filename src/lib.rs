//! MikroTik RIF Viewer — library surface.
//!
//! The product is a single cross-platform desktop application, but the capture
//! reader is also exposed as a library so that integration tests, fuzz targets,
//! benchmarks and the headless CLI can exercise it without pulling in the GUI.
//! The parser itself is UI-agnostic and free of filesystem access.
//!
//! The `run` entry point starts the desktop shell; the binary in `src/main.rs`
//! is only a thin wrapper around it.

#![forbid(unsafe_code)]

// The parser is the only module with a stable, externally consumed surface
// (integration tests, `fuzz/`, `benches/` and the CLI). Everything else stays
// private to the crate and is reached through [`run`].
pub mod parser;

mod app;
mod build_info;
mod cli;
mod i18n;
mod icons;
mod panic;
mod splash;
mod theme;
mod update;
mod worker;

use eframe::egui;

pub use parser::PRODUCT_NAME;

/// Start the viewer, or perform a headless CLI operation when the process was
/// invoked with one.
///
/// Returns the eframe result for the GUI path. CLI paths terminate the process
/// themselves and never reach the window.
pub fn run() -> eframe::Result {
    panic::install();

    if let Some(code) = cli::dispatch() {
        std::process::exit(code);
    }

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
