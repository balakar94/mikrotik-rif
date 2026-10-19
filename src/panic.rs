//! Last-resort panic capture: log to a file and surface a visible dialog.
//!
//! A GUI build has nowhere to print a panic: on release builds for Windows
//! there is not even a console, so without this module the user only sees the
//! window vanish. [`install`] registers a hook that records the panic message
//! and location in memory and appends the same entry to a log file in the
//! system temporary directory. The interface thread keeps running for panics
//! raised on background threads, so the viewer polls [`take_report`] once per
//! frame and renders [`show_dialog`] while a report is pending.
//!
//! The dialog deliberately uses hard-coded English literals instead of Fluent
//! identifiers: it is a last-resort UI that must render even when localization
//! is broken, and adding identifiers would break the
//! `every_locale_defines_the_base_identifiers` test, which requires every
//! locale to define exactly the same set of identifiers.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use eframe::egui;

/// Name of the append-only panic log inside the system temporary directory.
const LOG_FILE_NAME: &str = "mikrotik-rif-panic.log";

/// Upper bound for the panic log. A panic loop would otherwise fill the disk;
/// the file is truncated before the next entry once it grows past this.
const MAX_LOG_BYTES: u64 = 256 * 1024;

/// In-memory slot for the most recent panic. Drained by the UI thread.
static PENDING: Mutex<Option<PanicReport>> = Mutex::new(None);

/// A captured panic: what happened, where, and where it was logged.
#[derive(Clone, Debug)]
pub struct PanicReport {
    message: String,
    location: String,
    log_path: PathBuf,
}

impl PanicReport {
    /// One line per field, suitable for the clipboard.
    fn details(&self) -> String {
        format!(
            "message: {}\nlocation: {}\nlog: {}",
            self.message,
            self.location,
            self.log_path.display()
        )
    }
}

/// Register the panic hook. Call once at startup, before `eframe::run_native`.
///
/// The hook itself never panics and never blocks the unwind: lock and I/O
/// failures are silently ignored because there is nothing sensible to do with
/// them while already handling a panic.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let message = message_of(info);
        let location = info
            .location()
            .map_or_else(|| "unknown location".to_owned(), ToString::to_string);
        let log_path = log_path();
        append_log(&log_path, &message, &location);
        store(PanicReport {
            message,
            location,
            log_path,
        });
    }));
}

/// Take the pending panic report, if any. The viewer calls this once per frame.
#[must_use]
pub fn take_report() -> Option<PanicReport> {
    PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
}

/// Render the panic dialog as a centered, non-collapsible window.
///
/// Returns `true` once the user presses Close, so the caller can dismiss the
/// stored report. All strings are hard-coded English on purpose; see the
/// module documentation.
#[must_use]
pub fn show_dialog(ctx: &egui::Context, report: &PanicReport) -> bool {
    let mut dismissed = false;
    egui::Window::new("Unexpected error")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.label("Something went wrong. The details below were also saved to the log file.");
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(report.message.as_str()).monospace());
                });
            ui.add_space(6.0);
            ui.label(format!("Log file: {}", report.log_path.display()));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Copy details").clicked() {
                    ctx.copy_text(report.details());
                }
                if ui.button("Close").clicked() {
                    dismissed = true;
                }
            });
        });
    dismissed
}

/// Path of the append-only panic log in the system temporary directory.
fn log_path() -> PathBuf {
    std::env::temp_dir().join(LOG_FILE_NAME)
}

/// Extract a message from a panic payload, whatever its type is.
fn message_of(info: &std::panic::PanicHookInfo<'_>) -> String {
    message_of_payload(info.payload())
}

/// [`message_of`] over a borrowed payload, so the extraction rule is testable
/// without fabricating a [`std::panic::PanicHookInfo`].
fn message_of_payload(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

/// Remember the report for the UI thread. Lock poisoning is recovered from
/// because losing the previous report is better than losing this one.
fn store(report: PanicReport) {
    let mut pending = PENDING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *pending = Some(report);
}

/// Append one entry to the panic log. Every failure is ignored: this runs
/// while unwinding, so there is no one left to report an error to.
fn append_log(path: &Path, message: &str, location: &str) {
    let _ = append_log_to(path, message, location);
}

/// Append one entry to `path`, creating it with owner-only permissions
/// (`0o600` on Unix) and truncating it first when it already exceeds
/// [`MAX_LOG_BYTES`].
///
/// The log can contain file paths and other details from the failing process,
/// so it is kept private to the user and bounded in size.
fn append_log_to(path: &Path, message: &str, location: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or_else(
            |_| "unknown-time".to_owned(),
            |elapsed| {
                let secs = elapsed.as_secs();
                format!("{secs}s since epoch")
            },
        );
    // Never follow a symlink at the log path: a pre-created link in the
    // shared temporary directory could otherwise redirect panic output into an
    // unrelated file. `append_log_file` creates the entry exclusively first
    // (which cannot follow a link), and only falls back to opening an existing
    // entry after confirming it is not one.
    let mut file = append_log_file(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    writeln!(file, "[{timestamp}] panic: {message} ({location})")
}

/// Open `path` for appending without ever following a symlink.
///
/// The first attempt uses `create_new`, which the kernel implements as
/// `O_CREAT|O_EXCL`: it fails with `AlreadyExists` when anything at all sits at
/// `path` — including a symlink, which `O_EXCL` refuses to follow — so a link
/// planted in advance cannot capture the entry. Only when the existing entry is
/// a regular file do we fall back to a plain append (truncating first when it
/// exceeded [`MAX_LOG_BYTES`]); a link is moved aside and the exclusive create
/// retried.
///
/// A link swapped in *after* the lstat and before the fallback `open` is still
/// caught: the handle is compared against the inode that was inspected, and a
/// mismatch fails the append instead of writing through the link. A lost log
/// entry is an acceptable price for never writing panic text into an unrelated
/// file.
fn append_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    match exclusive_append_open(path) {
        Ok(file) => return Ok(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    // Inspected without following the path, so the size test below cannot be
    // steered by a link target either.
    let existing = std::fs::symlink_metadata(path)?;
    if existing.file_type().is_symlink() {
        let mut backup = path.as_os_str().to_owned();
        backup.push(".symlink-bak");
        let _ = std::fs::rename(path, PathBuf::from(backup));
        return exclusive_append_open(path);
    }
    // Truncation is done by reopening the file rather than with
    // `File::set_len`: on Windows the append-only handle does not carry the
    // access right that resizing requires, so `set_len` fails there.
    let oversized = existing.len() > MAX_LOG_BYTES;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(oversized)
        .append(!oversized)
        .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        // Same device and inode as the entry we inspected: the handle is the
        // file we meant to append to and not the target of a link swapped in
        // between the two syscalls.
        let opened = file.metadata()?;
        if (existing.dev(), existing.ino()) != (opened.dev(), opened.ino()) {
            return Err(std::io::Error::other(
                "panic log path was replaced while it was being opened",
            ));
        }
    }
    Ok(file)
}

/// Create `path` exclusively (Unix: mode `0o600`) and open it for appending.
fn exclusive_append_open(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_text_handles_str_string_and_other_types() {
        let borrowed: &str = "borrowed panic";
        assert_eq!(message_of_payload(&borrowed), "borrowed panic");
        assert_eq!(
            message_of_payload(&String::from("owned panic")),
            "owned panic"
        );
        assert_eq!(message_of_payload(&42_u32), "unknown panic payload");
    }

    #[test]
    fn report_round_trips_through_the_slot() {
        store(PanicReport {
            message: "boom".to_owned(),
            location: "file.rs:1:1".to_owned(),
            log_path: PathBuf::from("/tmp/example.log"),
        });
        let taken = take_report().expect("the stored report is returned");
        assert_eq!(taken.message, "boom");
        assert!(take_report().is_none(), "the slot is drained after a take");
    }

    #[test]
    fn log_appends_then_truncates_when_oversized() {
        use std::io::Read as _;

        let dir =
            std::env::temp_dir().join(format!("mikrotik-rif-panic-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let log = dir.join("panic.log");

        append_log_to(&log, "first", "a.rs:1").expect("first append");
        append_log_to(&log, "second", "b.rs:2").expect("second append");
        let body = std::fs::read_to_string(&log).expect("read log");
        assert!(body.contains("first"), "previous entry is kept");
        assert!(body.contains("second"), "new entry is appended");

        // An oversized log is truncated before the next entry is written.
        let mut source = std::io::repeat(b'x').take(MAX_LOG_BYTES + 1);
        let mut sink = std::fs::File::create(&log).expect("oversize log");
        std::io::copy(&mut source, &mut sink).expect("write oversize log");
        drop(sink);
        append_log_to(&log, "after-truncate", "c.rs:3").expect("append after truncate");
        let body = std::fs::read_to_string(&log).expect("read log");
        assert!(body.contains("after-truncate"));
        assert!(
            body.len() < 1024,
            "log was truncated to a single entry: {} bytes",
            body.len()
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn log_never_writes_through_a_symlink() {
        let dir =
            std::env::temp_dir().join(format!("mikrotik-rif-panic-symlink-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let target = dir.join("target.txt");
        let log = dir.join("panic.log");
        let _ = std::fs::remove_file(&log);
        let mut backup = log.as_os_str().to_owned();
        backup.push(".symlink-bak");
        let _ = std::fs::remove_file(PathBuf::from(&backup));
        std::fs::write(&target, "original").expect("target file");

        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &log).expect("link log");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&target, &log).expect("link log");

        append_log_to(&log, "symlink-probe", "s.rs:1").expect("append");

        let victim = std::fs::read_to_string(&target).expect("read");
        assert!(!victim.contains("symlink-probe"), "link target kept");
        assert!(
            !std::fs::symlink_metadata(&log)
                .expect("log metadata")
                .file_type()
                .is_symlink(),
            "log path is a fresh file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn log_file_is_created_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir =
            std::env::temp_dir().join(format!("mikrotik-rif-panic-mode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let log = dir.join("panic.log");

        append_log_to(&log, "mode-probe", "m.rs:1").expect("append");
        let mode = std::fs::metadata(&log)
            .expect("log metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the log stays private to the user");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
