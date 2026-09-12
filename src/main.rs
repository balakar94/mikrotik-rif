//! MikroTik RIF Viewer — single cross-platform desktop application.
//!
//! The whole product lives in this one package: the `parser` module reads the
//! capture format, and the UI modules (`app`, `splash`, `theme`, `worker`) render
//! it. All work happens in-process; there is no network path anywhere.

#![forbid(unsafe_code)]

mod app;
mod i18n;
mod icons;
// The parser is an internal, self-contained library. Part of its surface is
// exercised only by tests or reserved for upcoming features, so dead-code
// analysis is relaxed for this module only.
#[allow(dead_code)]
mod parser;
mod splash;
mod theme;
mod worker;

use eframe::egui;
use parser::PRODUCT_NAME;

fn main() -> eframe::Result {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon/icon.png"))
        .unwrap_or_default();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(PRODUCT_NAME)
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
