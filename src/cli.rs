//! Headless command-line surface for the capture reader.
//!
//! The desktop viewer is the default entry point; this module only takes over
//! when the process is invoked with a recognised subcommand, so double-clicking
//! a `.rif` (which passes a bare path) still launches the GUI.
//!
//! The argument parser is deliberately a pure function over the arguments after
//! the program name, so it can be unit-tested without touching the filesystem,
//! stdout or a capture. [`dispatch`] is the only side-effecting entry point: it
//! reads `std::env::args_os()`, runs the requested command and returns the exit
//! code the caller (`crate::run`) must use.
//!
//! Owned by the CLI stream; the full implementation lives here.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::filenames::{escape_label, sanitize};
use crate::parser::{CODE_NAME, Capture, CaptureLimits};
use crate::worker::MAX_CAPTURE_BYTES;

/// Successful completion.
const EXIT_OK: i32 = 0;
/// The command was recognised but the operation failed (missing file, not a
/// capture, unreadable module, IO error).
const EXIT_FAILURE: i32 = 1;
/// The command line itself was malformed.
const EXIT_USAGE: i32 = 2;

/// Dispatch a headless command when one was requested.
///
/// Returns `Some(exit_code)` when the process must terminate without starting
/// the GUI, or `None` to fall through to the desktop shell. A bare path and any
/// other unrecognised first argument return `None`, so double-clicking a
/// capture keeps opening the viewer.
#[must_use]
pub(crate) fn dispatch() -> Option<i32> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match parse_args(&args) {
        Ok(None) => None,
        Ok(Some(command)) => Some(run(command)),
        Err(error) => {
            eprintln!("{CODE_NAME}: {error}");
            eprintln!("Try '{CODE_NAME} --help' for usage.");
            Some(EXIT_USAGE)
        }
    }
}

// ── Pure argument model ──────────────────────────────────────────────────────

/// A recognised headless command.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    /// Print usage and exit successfully.
    Help,
    /// List every indexed part.
    List {
        /// Capture file to open.
        capture: PathBuf,
    },
    /// Expand one or more modules.
    Extract(ExtractRequest),
}

/// Everything `--extract` needs.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ExtractRequest {
    /// Capture file to open.
    capture: PathBuf,
    /// Which modules to expand.
    selection: Selection,
    /// Where expanded text goes.
    destination: Destination,
}

/// Which parts `--extract` selects.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Selection {
    /// Every readable part.
    All,
    /// The first part whose label matches exactly.
    Module(String),
}

/// Where `--extract` writes expanded text.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Destination {
    /// `<capture-stem>.parts/` beside the capture.
    DefaultDirectory,
    /// The directory named by `--out`.
    Directory(PathBuf),
    /// Standard output.
    Stdout,
}

/// A malformed command line. Rendered to stderr and followed by an exit code of
/// [`EXIT_USAGE`].
#[derive(Debug)]
struct UsageError(String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Parse the arguments after the program name.
///
/// `Ok(None)` means the process was not invoked with a recognised subcommand
/// (a bare capture path, no arguments, a leading `--`, …): the caller must start
/// the GUI. `Err` means a recognised subcommand carried an invalid command line.
fn parse_args(args: &[OsString]) -> Result<Option<Command>, UsageError> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    // A leading `--` ends option processing at the top level; everything after
    // it is positional, so nothing recognises a subcommand and the GUI starts.
    if first.as_os_str() == OsStr::new("--") {
        return Ok(None);
    }
    let Some(token) = first.to_str() else {
        // A non-UTF-8 first argument can only be a path: let the GUI open it.
        return Ok(None);
    };
    match token {
        "-h" | "--help" => Ok(Some(Command::Help)),
        "--list" => parse_list(&args[1..]).map(Some),
        "--extract" => parse_extract(&args[1..]).map(Some),
        _ => Ok(None),
    }
}

/// Parse `--list [--] <capture>`.
fn parse_list(args: &[OsString]) -> Result<Command, UsageError> {
    let mut capture: Option<PathBuf> = None;
    let mut options_ended = false;
    for arg in args {
        if !options_ended && arg.as_os_str() == OsStr::new("--") {
            options_ended = true;
            continue;
        }
        if !options_ended
            && let Some(token) = arg.to_str()
            && token.starts_with('-')
            && token != "-"
        {
            return Err(UsageError(format!("unknown option {token:?} for --list")));
        }
        if capture.is_some() {
            return Err(UsageError(
                "--list accepts exactly one capture path".to_owned(),
            ));
        }
        capture = Some(PathBuf::from(arg));
    }
    let Some(capture) = capture else {
        return Err(UsageError("--list requires a capture path".to_owned()));
    };
    Ok(Command::List { capture })
}

/// Parse `--extract [--] <capture> [--module <label>] [--out <dir>] [--stdout] [--all]`.
fn parse_extract(args: &[OsString]) -> Result<Command, UsageError> {
    let mut capture: Option<PathBuf> = None;
    let mut module: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut stdout = false;
    let mut all = false;
    let mut options_ended = false;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        index += 1;
        if !options_ended {
            if arg.as_os_str() == OsStr::new("--") {
                options_ended = true;
                continue;
            }
            if let Some(token) = arg.to_str() {
                match token {
                    "--module" => {
                        if module.is_some() {
                            return Err(UsageError("--module may be given only once".to_owned()));
                        }
                        let value = take_value(args, &mut index, "--module")?;
                        let label = value.to_str().ok_or_else(|| {
                            UsageError("--module label must be valid UTF-8".to_owned())
                        })?;
                        module = Some(label.to_owned());
                        continue;
                    }
                    "--out" => {
                        if out.is_some() {
                            return Err(UsageError("--out may be given only once".to_owned()));
                        }
                        let value = take_value(args, &mut index, "--out")?;
                        out = Some(PathBuf::from(value));
                        continue;
                    }
                    "--stdout" => {
                        stdout = true;
                        continue;
                    }
                    "--all" => {
                        all = true;
                        continue;
                    }
                    _ => {}
                }
                if token.starts_with('-') && token != "-" {
                    return Err(UsageError(format!(
                        "unknown option {token:?} for --extract"
                    )));
                }
            }
        }
        if capture.is_some() {
            return Err(UsageError(
                "--extract accepts exactly one capture path".to_owned(),
            ));
        }
        capture = Some(PathBuf::from(arg));
    }

    let capture =
        capture.ok_or_else(|| UsageError("--extract requires a capture path".to_owned()))?;

    let selection = match (all, module) {
        (true, Some(_)) => {
            return Err(UsageError(
                "--all and --module cannot be combined".to_owned(),
            ));
        }
        (true, None) => Selection::All,
        (false, Some(label)) => Selection::Module(label),
        (false, None) => {
            return Err(UsageError(
                "--extract requires --module <label> or --all".to_owned(),
            ));
        }
    };

    let destination = match (stdout, out) {
        (true, Some(_)) => {
            return Err(UsageError(
                "--stdout and --out cannot be combined".to_owned(),
            ));
        }
        (true, None) => Destination::Stdout,
        (false, Some(dir)) => Destination::Directory(dir),
        (false, None) => Destination::DefaultDirectory,
    };

    Ok(Command::Extract(ExtractRequest {
        capture,
        selection,
        destination,
    }))
}

/// Consume the value that follows a value-taking option.
fn take_value(args: &[OsString], index: &mut usize, option: &str) -> Result<OsString, UsageError> {
    let Some(value) = args.get(*index) else {
        return Err(UsageError(format!("{option} requires a value")));
    };
    *index += 1;
    Ok(value.clone())
}

// ── Command execution ────────────────────────────────────────────────────────

/// An operational failure: the command was well formed but could not complete.
#[derive(Debug)]
struct CliError(String);

impl CliError {
    /// Build an operational failure from a human-readable message.
    fn operational(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Whether an output stream is still usable after a write.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    /// The stream accepted the bytes.
    Open,
    /// The reader closed the pipe; further writes are pointless but not errors.
    Closed,
}

/// Run a parsed command and return its exit code.
fn run(command: Command) -> i32 {
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let result = match command {
        Command::Help => write_help(&mut out),
        Command::List { capture } => list(&capture, &mut out),
        Command::Extract(request) => extract(&request, &mut out),
    };
    match result {
        Ok(()) => EXIT_OK,
        Err(error) => report(&error),
    }
}

/// Print an operational failure and return the matching exit code.
fn report(error: &CliError) -> i32 {
    eprintln!("{CODE_NAME}: {error}");
    EXIT_FAILURE
}

/// Write usage to `out`. A reader that closed the pipe is not an error.
fn write_help<W: Write>(out: &mut W) -> Result<(), CliError> {
    let help = format!(
        "{CODE_NAME} — headless capture reader\n\
         \n\
         Usage:\n\
         \x20 {CODE_NAME} --help | -h\n\
         \x20 {CODE_NAME} --list <capture.rif>\n\
         \x20 {CODE_NAME} --extract <capture.rif> --module <label> [--out <dir> | --stdout]\n\
         \x20 {CODE_NAME} --extract <capture.rif> --all [--out <dir> | --stdout]\n\
         \n\
         Commands:\n\
         \x20 -h, --help            Print this help and exit.\n\
         \x20     --list            List every part: ordinal, label, compressed bytes, status.\n\
         \x20     --extract         Expand module text.\n\
         \n\
         Extract options:\n\
         \x20     --module <label>  Expand the first part whose label matches exactly.\n\
         \x20     --all             Expand every readable part.\n\
         \x20     --out <dir>       Write expanded text as <dir>/<sanitised-label>.txt.\n\
         \x20     --stdout          Write expanded text to standard output.\n\
         \n\
         Other:\n\
         \x20     --                Treat the next argument as a path, even if it starts with '-'.\n\
         \n\
         Without --out or --stdout, text is written to '<capture-stem>.parts/' beside\n\
         the capture. A bare path starts the desktop viewer instead.\n"
    );
    if write_bytes(out, help.as_bytes())? == Stream::Closed {
        return Ok(());
    }
    flush_output(out)?;
    Ok(())
}

/// Read and index a capture, mapping every failure to an operational error.
///
/// Files larger than [`MAX_CAPTURE_BYTES`] are rejected up front from the file
/// metadata so an oversized capture never reaches the in-memory buffer; a file
/// that grows between the metadata check and the read is rejected again from
/// the buffered length. The reason mirrors the worker's `TOO_LARGE_REASON`.
fn load_capture(path: &Path) -> Result<Capture, CliError> {
    if let Ok(metadata) = std::fs::metadata(path)
        && metadata.len() > MAX_CAPTURE_BYTES
    {
        return Err(CliError::operational(format!(
            "{} is not a readable capture: file too large (>512 MiB)",
            path.display()
        )));
    }
    let bytes = std::fs::read(path).map_err(|error| {
        CliError::operational(format!("cannot read {}: {error}", path.display()))
    })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CAPTURE_BYTES {
        return Err(CliError::operational(format!(
            "{} is not a readable capture: file too large (>512 MiB)",
            path.display()
        )));
    }
    let capture = Capture::from_bytes(&bytes, &CaptureLimits::default()).map_err(|error| {
        CliError::operational(format!(
            "{} is not a readable capture: {error}",
            path.display()
        ))
    })?;
    if capture.is_empty() {
        return Err(CliError::operational(format!(
            "{} is not a capture (no parts found)",
            path.display()
        )));
    }
    Ok(capture)
}

/// Print one tab-separated line per part to `out`.
///
/// Labels come from the capture file and are untrusted: control characters
/// (newlines, ANSI escapes) are neutralised with [`escape_label`] so a
/// hostile label cannot inject lines or terminal sequences into the listing.
/// The extracted content itself is never altered here.
fn list<W: Write>(path: &Path, out: &mut W) -> Result<(), CliError> {
    let capture = load_capture(path)?;
    let mut listing = String::new();
    for part in capture.parts() {
        let status = match part.fault() {
            Some(fault) => format!("unreadable: {fault}"),
            None => "ok".to_owned(),
        };
        writeln!(
            listing,
            "{}\t{}\t{}\t{}",
            part.ordinal(),
            escape_label(part.label()),
            part.compressed_len(),
            status
        )
        .expect("writing into a String never fails");
    }
    if write_bytes(out, listing.as_bytes())? == Stream::Open {
        flush_output(out)?;
    }
    Ok(())
}

/// Expand the selected modules into their destination, reporting written paths
/// to `out` for the directory destinations.
fn extract<W: Write>(request: &ExtractRequest, out: &mut W) -> Result<(), CliError> {
    let capture = load_capture(&request.capture)?;
    let limits = CaptureLimits::default();
    let indices = selected_indices(&capture, &request.selection)?;
    match &request.destination {
        Destination::Stdout => extract_to_stdout(&capture, &indices, &limits, out),
        Destination::Directory(directory) => {
            extract_to_directory(&capture, &indices, &limits, directory, out)
        }
        Destination::DefaultDirectory => {
            let directory = default_parts_directory(&request.capture)?;
            extract_to_directory(&capture, &indices, &limits, &directory, out)
        }
    }
}

/// Resolve the selection to a non-empty list of readable part indices.
fn selected_indices(capture: &Capture, selection: &Selection) -> Result<Vec<usize>, CliError> {
    match selection {
        Selection::Module(label) => {
            let Some(index) = capture.find_first_named(label) else {
                return Err(CliError::operational(format!("no module named {label:?}")));
            };
            let part = &capture.parts()[index];
            if !part.is_readable() {
                return Err(CliError::operational(format!(
                    "module {label:?} is unreadable: {}",
                    part.fault().unwrap_or("unknown fault")
                )));
            }
            Ok(vec![index])
        }
        Selection::All => {
            let mut indices = Vec::new();
            for (index, part) in capture.parts().iter().enumerate() {
                if part.is_readable() {
                    indices.push(index);
                } else {
                    eprintln!(
                        "{CODE_NAME}: skipping unreadable module {:?}: {}",
                        part.label(),
                        part.fault().unwrap_or("unknown fault")
                    );
                }
            }
            if indices.is_empty() {
                return Err(CliError::operational(
                    "capture holds no readable modules".to_owned(),
                ));
            }
            Ok(indices)
        }
    }
}

/// Expand every selected module to `out`, separating multiple documents with a
/// header so the output stays unambiguous.
///
/// The label in the header is untrusted capture input, so it is passed through
/// [`escape_label`] to neutralise newlines and ANSI escapes that could forge
/// headers or drive the terminal. The expanded body is written verbatim.
fn extract_to_stdout<W: Write>(
    capture: &Capture,
    indices: &[usize],
    limits: &CaptureLimits,
    out: &mut W,
) -> Result<(), CliError> {
    let with_headers = indices.len() > 1;
    for &index in indices {
        let part = &capture.parts()[index];
        let text = capture.read(index, limits).map_err(|error| {
            CliError::operational(format!("cannot expand module {:?}: {error}", part.label()))
        })?;
        // Two sequential writes keep the byte stream identical to the old
        // single `chunk` write while avoiding a second copy of the expanded
        // text (up to 256 MiB per part) in an intermediate `String`.
        if with_headers {
            let header = format!(
                "\n===== {} [{}] =====\n",
                escape_label(part.label()),
                part.ordinal()
            );
            if write_bytes(out, header.as_bytes())? == Stream::Closed {
                return Ok(());
            }
        }
        if write_bytes(out, text.text.as_bytes())? == Stream::Closed {
            return Ok(());
        }
    }
    flush_output(out)?;
    Ok(())
}

/// Expand every selected module into `directory`, one sanitised file per part,
/// printing the path of each file that was written to `out`.
///
/// Hardening: each file is created with `create_new`, so a pre-existing
/// regular file is never truncated — the writer falls through to the next
/// `unique_stem` suffix (`<stem>-2.txt`, …). A final path that already exists
/// as a symlink is refused with an operational error instead of being
/// followed. Only the final file path is checked: parent components of
/// `directory` are created with `create_dir_all` and are not inspected for
/// symlinks, so callers must pass a trustworthy destination.
fn extract_to_directory<W: Write>(
    capture: &Capture,
    indices: &[usize],
    limits: &CaptureLimits,
    directory: &Path,
    out: &mut W,
) -> Result<(), CliError> {
    std::fs::create_dir_all(directory).map_err(|error| {
        CliError::operational(format!(
            "cannot create directory {}: {error}",
            directory.display()
        ))
    })?;

    let mut used = HashSet::new();
    let mut stream = Stream::Open;

    for &index in indices {
        let part = &capture.parts()[index];
        let text = capture.read(index, limits).map_err(|error| {
            CliError::operational(format!("cannot expand module {:?}: {error}", part.label()))
        })?;
        let mut stem = unique_stem(part.label(), &mut used);
        let path = loop {
            let candidate = directory.join(format!("{stem}.txt"));
            if let Ok(metadata) = std::fs::symlink_metadata(&candidate)
                && metadata.file_type().is_symlink()
            {
                return Err(CliError::operational(format!(
                    "refusing to overwrite symlink {}",
                    candidate.display()
                )));
            }
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(mut file) => {
                    file.write_all(text.text.as_bytes()).map_err(|error| {
                        CliError::operational(format!(
                            "cannot write {}: {error}",
                            candidate.display()
                        ))
                    })?;
                    break candidate;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    stem = unique_stem(part.label(), &mut used);
                }
                Err(error) => {
                    return Err(CliError::operational(format!(
                        "cannot write {}: {error}",
                        candidate.display()
                    )));
                }
            }
        };
        if stream == Stream::Open {
            let line = format!("{}\n", path.display());
            stream = write_bytes(out, line.as_bytes())?;
        }
    }

    if stream == Stream::Open {
        flush_output(out)?;
    }
    Ok(())
}

/// Derive the default `<capture-stem>.parts/` directory beside the capture.
fn default_parts_directory(capture: &Path) -> Result<PathBuf, CliError> {
    let Some(stem) = capture.file_stem() else {
        return Err(CliError::operational(format!(
            "cannot derive a parts directory from {}",
            capture.display()
        )));
    };
    let mut name = stem.to_os_string();
    name.push(".parts");
    Ok(capture.with_file_name(name))
}

/// Pick a file-name stem for `label` that has not been used yet.
///
/// The first part with a given sanitised label keeps the plain stem; later
/// collisions get a numeric suffix so `--all` never silently overwrites a file.
fn unique_stem(label: &str, used: &mut HashSet<String>) -> String {
    let base = sanitize(label);
    if used.insert(base.clone()) {
        return base;
    }
    let mut counter = 2_u32;
    loop {
        let candidate = format!("{base}-{counter}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        counter = counter.saturating_add(1);
    }
}

/// Write to an output stream, treating a closed reader as a clean stop.
fn write_bytes<W: Write>(out: &mut W, bytes: &[u8]) -> Result<Stream, CliError> {
    match out.write_all(bytes) {
        Ok(()) => Ok(Stream::Open),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(Stream::Closed),
        Err(error) => Err(CliError::operational(format!(
            "cannot write output: {error}"
        ))),
    }
}

/// Flush an output stream, treating a closed reader as a clean stop.
fn flush_output<W: Write>(out: &mut W) -> Result<Stream, CliError> {
    match out.flush() {
        Ok(()) => Ok(Stream::Open),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(Stream::Closed),
        Err(error) => Err(CliError::operational(format!(
            "cannot flush output: {error}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use crate::parser::codec;
    use crate::parser::scanner::{CLOSE_MARKER, OPEN_MARKER};

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_falls_through_to_the_gui() {
        assert_eq!(parse_args(&args(&[])).unwrap(), None);
    }

    #[test]
    fn bare_path_falls_through_to_the_gui() {
        assert_eq!(parse_args(&args(&["capture.rif"])).unwrap(), None);
        assert_eq!(parse_args(&args(&["/tmp/capture.rif"])).unwrap(), None);
    }

    #[test]
    fn leading_separator_falls_through_to_the_gui() {
        assert_eq!(parse_args(&args(&["--", "--list", "a.rif"])).unwrap(), None);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_bare_path_falls_through_to_the_gui() {
        use std::os::unix::ffi::OsStringExt as _;
        let path = OsString::from_vec(vec![b'c', 0xff, b'.', b'r', b'i', b'f']);
        assert_eq!(parse_args(&[path]).unwrap(), None);
    }

    #[test]
    fn help_flags_are_recognised() {
        assert_eq!(parse_args(&args(&["--help"])).unwrap(), Some(Command::Help));
        assert_eq!(parse_args(&args(&["-h"])).unwrap(), Some(Command::Help));
    }

    #[test]
    fn list_parses_one_capture() {
        assert_eq!(
            parse_args(&args(&["--list", "a.rif"])).unwrap(),
            Some(Command::List {
                capture: PathBuf::from("a.rif")
            })
        );
    }

    #[test]
    fn list_accepts_a_double_dash_before_a_dashed_path() {
        assert_eq!(
            parse_args(&args(&["--list", "--", "-dash.rif"])).unwrap(),
            Some(Command::List {
                capture: PathBuf::from("-dash.rif")
            })
        );
    }

    #[test]
    fn list_rejects_missing_extra_and_unknown_arguments() {
        assert!(parse_args(&args(&["--list"])).is_err());
        assert!(parse_args(&args(&["--list", "a.rif", "b.rif"])).is_err());
        assert!(parse_args(&args(&["--list", "--bogus"])).is_err());
    }

    #[test]
    fn extract_module_parses_all_options() {
        assert_eq!(
            parse_args(&args(&[
                "--extract",
                "cap.rif",
                "--module",
                "ip/firewall",
                "--out",
                "parts",
            ]))
            .unwrap(),
            Some(Command::Extract(ExtractRequest {
                capture: PathBuf::from("cap.rif"),
                selection: Selection::Module("ip/firewall".to_owned()),
                destination: Destination::Directory(PathBuf::from("parts")),
            }))
        );
    }

    #[test]
    fn extract_all_defaults_to_the_sibling_parts_directory() {
        assert_eq!(
            parse_args(&args(&["--extract", "cap.rif", "--all"])).unwrap(),
            Some(Command::Extract(ExtractRequest {
                capture: PathBuf::from("cap.rif"),
                selection: Selection::All,
                destination: Destination::DefaultDirectory,
            }))
        );
    }

    #[test]
    fn extract_stdout_is_recognised() {
        assert_eq!(
            parse_args(&args(&[
                "--extract",
                "cap.rif",
                "--module",
                "log",
                "--stdout"
            ]))
            .unwrap(),
            Some(Command::Extract(ExtractRequest {
                capture: PathBuf::from("cap.rif"),
                selection: Selection::Module("log".to_owned()),
                destination: Destination::Stdout,
            }))
        );
    }

    #[test]
    fn extract_requires_a_selector() {
        assert!(parse_args(&args(&["--extract", "cap.rif"])).is_err());
    }

    #[test]
    fn extract_rejects_conflicting_and_duplicate_options() {
        assert!(parse_args(&args(&["--extract", "cap.rif", "--all", "--module", "log"])).is_err());
        assert!(
            parse_args(&args(&[
                "--extract",
                "cap.rif",
                "--all",
                "--stdout",
                "--out",
                "."
            ]))
            .is_err()
        );
        assert!(
            parse_args(&args(&[
                "--extract",
                "cap.rif",
                "--all",
                "--module",
                "a",
                "--module",
                "b"
            ]))
            .is_err()
        );
        assert!(
            parse_args(&args(&[
                "--extract",
                "cap.rif",
                "--out",
                "a",
                "--out",
                "b",
                "--all"
            ]))
            .is_err()
        );
    }

    #[test]
    fn extract_rejects_missing_values_and_unknown_options() {
        assert!(parse_args(&args(&["--extract", "cap.rif", "--all", "--out"])).is_err());
        assert!(parse_args(&args(&["--extract", "cap.rif", "--all", "--module"])).is_err());
        assert!(parse_args(&args(&["--extract", "cap.rif", "--all", "--bogus"])).is_err());
        assert!(parse_args(&args(&["--extract", "--all"])).is_err());
        // `--` ends option processing: `--all` after it is a second path.
        assert!(parse_args(&args(&["--extract", "--", "-cap.rif", "--all"])).is_err());
    }

    #[test]
    fn extract_accepts_a_double_dash_before_a_dashed_path() {
        assert_eq!(
            parse_args(&args(&["--extract", "--all", "--", "-cap.rif"])).unwrap(),
            Some(Command::Extract(ExtractRequest {
                capture: PathBuf::from("-cap.rif"),
                selection: Selection::All,
                destination: Destination::DefaultDirectory,
            }))
        );
    }

    #[test]
    fn unknown_first_argument_is_not_a_command() {
        for value in ["--frobnicate", "-x", "--", "-"] {
            assert_eq!(parse_args(&args(&[value])).unwrap(), None, "{value}");
        }
    }

    #[test]
    fn sanitize_matches_the_workspace_rules() {
        assert_eq!(sanitize("/ip/firewall/filter"), "ip_firewall_filter");
        assert_eq!(sanitize("log"), "log");
        assert_eq!(sanitize("a\x00b\x1Fc"), "abc");
        assert_eq!(sanitize("name...   "), "name");
        assert_eq!(sanitize("  ___  "), "module");
        assert_eq!(sanitize("/"), "module");
        let long = "x".repeat(100);
        assert_eq!(sanitize(&long).chars().count(), 64);
        for reserved in ["CON", "con", "PRN", "AUX", "NUL", "COM1", "com9", "LPT1"] {
            assert_eq!(sanitize(reserved), "module", "{reserved}");
        }
        assert_eq!(sanitize("CON.txt"), "module");
        assert_ne!(sanitize("console"), "module");
    }

    #[test]
    fn unique_stem_disambiguates_collisions() {
        let mut used = HashSet::new();
        assert_eq!(unique_stem("log", &mut used), "log");
        assert_eq!(unique_stem("log", &mut used), "log-2");
        assert_eq!(unique_stem("log", &mut used), "log-3");
        // Two distinct labels can sanitise to the same stem.
        assert_eq!(unique_stem("/x", &mut used), "x");
        assert_eq!(unique_stem("\\x", &mut used), "x-2");
    }

    #[test]
    fn default_parts_directory_rejects_stemless_path() {
        assert!(default_parts_directory(Path::new("..")).is_err());
        assert!(default_parts_directory(Path::new("/")).is_err());
    }

    #[test]
    fn unique_stem_handles_sanitised_collisions() {
        let mut used = HashSet::new();
        let stems: Vec<String> = ["a/b", "a\\b", "a:b", "a*b"]
            .iter()
            .map(|label| unique_stem(label, &mut used))
            .collect();
        assert_eq!(stems, ["a_b", "a_b-2", "a_b-3", "a_b-4"]);
    }

    // ── End-to-end IO ────────────────────────────────────────────────────────

    /// A unique, self-cleaning directory under the system temp root.
    struct TempDir(PathBuf);

    /// Monotonic counter keeping temp names unique across parallel tests.
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    impl TempDir {
        fn new(name: &str) -> Self {
            let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
            let path = std::env::temp_dir().join(format!(
                "mikrotik-rif-cli-{}-{sequence}-{name}",
                std::process::id()
            ));
            let _previous = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _cleanup = std::fs::remove_dir_all(&self.0);
        }
    }

    fn deflate(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).expect("deflate the part body");
        encoder.finish().expect("finish the zlib stream")
    }

    fn encode_part(label: &[u8], body: &[u8]) -> Vec<u8> {
        let mut plain = label.to_vec();
        plain.push(0);
        plain.extend_from_slice(&deflate(body));
        codec::pack(&plain)
    }

    fn wrap(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut source = Vec::new();
        for part in parts {
            source.extend_from_slice(OPEN_MARKER);
            source.push(b'\n');
            source.extend_from_slice(part);
            source.push(b'\n');
            source.extend_from_slice(CLOSE_MARKER);
            source.push(b'\n');
        }
        source
    }

    /// Write a capture with `parts` and return `(temp dir, capture path)`.
    fn capture_file(name: &str, parts: &[Vec<u8>]) -> (TempDir, PathBuf) {
        let dir = TempDir::new(name);
        let path = dir.path().join("capture.rif");
        std::fs::write(&path, wrap(parts)).expect("write capture");
        (dir, path)
    }

    #[test]
    fn list_reports_every_part_with_its_status() {
        let (_dir, path) = capture_file(
            "list",
            &[
                encode_part(b"alpha", b"hello\n"),
                // No NUL separator: indexed as an unreadable placeholder.
                codec::pack(b"no-separator"),
                encode_part(b"beta", b"bye\n"),
            ],
        );
        let mut output = Vec::new();
        list(&path, &mut output).expect("listing must succeed");
        let listing = String::from_utf8(output).expect("utf-8");
        let lines: Vec<&str> = listing.lines().collect();
        assert_eq!(lines.len(), 3, "{listing:?}");
        assert!(lines[0].starts_with("0\talpha\t"), "{listing:?}");
        assert!(lines[0].ends_with("\tok"), "{listing:?}");
        assert!(lines[1].contains("unreadable:"), "{listing:?}");
        assert!(lines[2].starts_with("0\tbeta\t"), "{listing:?}");
    }

    #[test]
    fn list_rejects_a_file_without_parts() {
        let dir = TempDir::new("not-a-capture");
        let path = dir.path().join("random.bin");
        std::fs::write(&path, b"not a capture").expect("write");
        let mut output = Vec::new();
        assert!(list(&path, &mut output).is_err());
    }

    #[test]
    fn extract_module_by_label_writes_one_file() {
        let (dir, path) = capture_file(
            "extract-module",
            &[
                encode_part(b"alpha", b"hello\n"),
                encode_part(b"../evil", b"rules\n"),
            ],
        );
        let out_dir = dir.path().join("parts");
        let request = ExtractRequest {
            capture: path,
            selection: Selection::Module("../evil".to_owned()),
            destination: Destination::Directory(out_dir.clone()),
        };
        let mut report = Vec::new();
        extract(&request, &mut report).expect("extract must succeed");
        // The path-traversal label is sanitised to `.._evil`.
        assert_eq!(
            std::fs::read_to_string(out_dir.join(".._evil.txt")).expect("file"),
            "rules\n"
        );
        assert!(String::from_utf8(report).unwrap().contains(".._evil.txt"));
    }

    #[test]
    fn extract_all_writes_each_readable_module_and_disambiguates_names() {
        let (dir, path) = capture_file(
            "extract-all",
            &[
                encode_part(b"dup", b"one\n"),
                encode_part(b"dup", b"two\n"),
                codec::pack(b"broken"),
            ],
        );
        let out_dir = dir.path().join("out");
        let request = ExtractRequest {
            capture: path,
            selection: Selection::All,
            destination: Destination::Directory(out_dir.clone()),
        };
        let mut report = Vec::new();
        extract(&request, &mut report).expect("extract must succeed");
        assert_eq!(
            std::fs::read_to_string(out_dir.join("dup.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(
            std::fs::read_to_string(out_dir.join("dup-2.txt")).unwrap(),
            "two\n"
        );
    }

    #[test]
    fn extract_stdout_writes_the_module_and_separates_multiple() {
        let (_dir, path) = capture_file(
            "extract-stdout",
            &[
                encode_part(b"alpha", b"hello\n"),
                encode_part(b"beta", b"bye\n"),
            ],
        );

        let single = ExtractRequest {
            capture: path.clone(),
            selection: Selection::Module("alpha".to_owned()),
            destination: Destination::Stdout,
        };
        let mut output = Vec::new();
        extract(&single, &mut output).expect("stdout extract");
        assert_eq!(String::from_utf8(output).unwrap(), "hello\n");

        let all = ExtractRequest {
            capture: path,
            selection: Selection::All,
            destination: Destination::Stdout,
        };
        let mut output = Vec::new();
        extract(&all, &mut output).expect("stdout extract all");
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("===== alpha [0] ====="), "{text:?}");
        assert!(text.contains("hello\n"), "{text:?}");
        assert!(text.contains("===== beta [0] ====="), "{text:?}");
        assert!(text.contains("bye\n"), "{text:?}");
    }

    #[test]
    fn extract_defaults_to_a_sibling_parts_directory() {
        let (dir, path) = capture_file("extract-default", &[encode_part(b"log", b"line\n")]);
        assert_eq!(
            default_parts_directory(&path).unwrap(),
            dir.path().join("capture.parts")
        );
        let request = ExtractRequest {
            capture: path,
            selection: Selection::Module("log".to_owned()),
            destination: Destination::DefaultDirectory,
        };
        let mut report = Vec::new();
        extract(&request, &mut report).expect("extract");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("capture.parts").join("log.txt")).unwrap(),
            "line\n"
        );
    }

    #[test]
    fn missing_module_is_an_operational_error() {
        let (_dir, path) = capture_file("missing", &[encode_part(b"log", b"line\n")]);
        let request = ExtractRequest {
            capture: path,
            selection: Selection::Module("nope".to_owned()),
            destination: Destination::Stdout,
        };
        let mut output = Vec::new();
        assert!(extract(&request, &mut output).is_err());
    }

    #[test]
    fn extract_never_truncates_a_preexisting_regular_file() {
        let (dir, path) = capture_file("no-truncate", &[encode_part(b"log", b"new\n")]);
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).expect("out dir");
        std::fs::write(out_dir.join("log.txt"), b"old\n").expect("sentinel");
        let request = ExtractRequest {
            capture: path,
            selection: Selection::Module("log".to_owned()),
            destination: Destination::Directory(out_dir.clone()),
        };
        let mut report = Vec::new();
        extract(&request, &mut report).expect("extract must succeed");
        assert_eq!(
            std::fs::read_to_string(out_dir.join("log.txt")).unwrap(),
            "old\n",
            "pre-existing file must be preserved"
        );
        assert_eq!(
            std::fs::read_to_string(out_dir.join("log-2.txt")).unwrap(),
            "new\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn extract_refuses_to_follow_a_preplanted_symlink() {
        let (dir, path) = capture_file("no-symlink", &[encode_part(b"log", b"new\n")]);
        let out_dir = dir.path().join("out");
        std::fs::create_dir_all(&out_dir).expect("out dir");
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, b"victim\n").expect("victim");
        std::os::unix::fs::symlink(&victim, out_dir.join("log.txt")).expect("symlink");
        let request = ExtractRequest {
            capture: path,
            selection: Selection::Module("log".to_owned()),
            destination: Destination::Directory(out_dir),
        };
        let mut report = Vec::new();
        let error = extract(&request, &mut report).expect_err("symlink must be refused");
        assert!(error.to_string().contains("symlink"), "{error}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "victim\n");
    }

    #[test]
    fn hostile_labels_are_neutralised_on_stdout_paths() {
        let hostile = "alpha\n===== beta\x1b[2J";
        let (_dir, path) = capture_file(
            "hostile-label",
            &[
                encode_part(hostile.as_bytes(), b"evil\n"),
                encode_part(b"other", b"plain\n"),
            ],
        );
        let escaped = escape_label(hostile);
        assert!(!escaped.contains('\n'));
        assert!(!escaped.contains('\x1b'));

        let mut listing = Vec::new();
        list(&path, &mut listing).expect("listing must succeed");
        let listing = String::from_utf8(listing).expect("utf-8");
        assert_eq!(listing.lines().count(), 2, "{listing:?}");
        assert!(listing.contains(&escaped), "{listing:?}");
        assert!(!listing.contains('\x1b'), "{listing:?}");

        let request = ExtractRequest {
            capture: path,
            selection: Selection::All,
            destination: Destination::Stdout,
        };
        let mut output = Vec::new();
        extract(&request, &mut output).expect("stdout extract");
        let text = String::from_utf8(output).expect("utf-8");
        assert!(text.contains(&escaped), "{text:?}");
        assert!(!text.contains('\x1b'), "{text:?}");
        // The escaped label keeps its inline `=====` run, but it sits
        // mid-line: only the two genuine headers start a line, so the
        // injected run cannot forge a header of its own.
        let header_lines = text
            .lines()
            .filter(|line| line.starts_with("====="))
            .count();
        assert_eq!(header_lines, 2, "{text:?}");
        assert!(text.contains("evil\n"), "body stays verbatim");
    }
}
