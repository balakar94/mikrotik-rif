//! MikroTik RIF Viewer — binary entry point.
//!
//! Every product module lives in the library crate (`src/lib.rs`) so that the
//! parser can also be consumed by integration tests, fuzz targets, benchmarks
//! and the headless CLI. This binary only applies the Windows subsystem
//! attribute and delegates to [`mikrotik_rif::run`].

#![forbid(unsafe_code)]
// GUI application on Windows: without this the linker assumes a console
// subsystem and Windows opens a terminal window next to the app. Gated on
// release builds so `cargo run` during development keeps stdout/stderr visible.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

fn main() -> eframe::Result {
    mikrotik_rif::run()
}
