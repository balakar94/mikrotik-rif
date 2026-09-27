//! End-to-end tests for the headless CLI surface.
//!
//! These tests compile the real binary and drive it through its process
//! boundary exactly as a user (or a packaging script) would, because the CLI is
//! the only externally observable interface that stands beside the GUI. The
//! behavioural source of truth is `src/cli.rs`; the fixtures here only recreate
//! its input format.
//!
//! Every capture is synthesised in memory with the inverse of the reader (label,
//! NUL separator, zlib payload, wrapped in the container markers) so no real
//! capture is ever committed: captures can carry sensitive router
//! configuration. Each test owns a unique, self-cleaning temporary directory to
//! stay independent under parallel execution.

use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::write::ZlibEncoder;
use mikrotik_rif::parser::codec;
use mikrotik_rif::parser::scanner::{CLOSE_MARKER, OPEN_MARKER};

/// Monotonic counter keeping temp directory names unique across parallel tests.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// A unique, self-cleaning directory under the system temp root.
struct TempDir(PathBuf);

impl TempDir {
    /// Create an empty directory whose name embeds this process and a sequence
    /// number, so two tests never share state.
    fn new(tag: &str) -> Self {
        let sequence = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!(
            "mikrotik-rif-cli-test-{}-{sequence}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create the temp directory");
        Self(path)
    }

    /// The directory path.
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compress `bytes` into the zlib payload a real part carries.
fn deflate(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes).expect("compress fixture bytes");
    encoder.finish().expect("finish fixture stream")
}

/// Encode one readable part the way the router would: label, NUL, zlib payload.
fn encode_part(label: &[u8], body: &[u8]) -> Vec<u8> {
    let mut plain = label.to_vec();
    plain.push(0);
    plain.extend_from_slice(&deflate(body));
    codec::pack(&plain)
}

/// Wrap already-encoded bodies in the marker pair.
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

/// Write `parts` to `capture.rif` inside a fresh temp directory.
fn capture_file(tag: &str, parts: &[Vec<u8>]) -> (TempDir, PathBuf) {
    let dir = TempDir::new(tag);
    let path = dir.path().join("capture.rif");
    std::fs::write(&path, wrap(parts)).expect("write the synthetic capture");
    (dir, path)
}

/// Run the compiled binary with `args` and capture its complete output.
fn cli(args: &[&OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mikrotik-rif"))
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the compiled binary must run")
}

/// Decode a captured stream for assertion messages.
fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn help_flags_print_usage_and_exit_zero() {
    for flag in ["--help", "-h"] {
        let output = cli(&[OsStr::new(flag)]);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{flag}: stderr {}",
            text(&output.stderr)
        );
        assert!(!output.stdout.is_empty(), "{flag}: usage must be printed");
        let stdout = text(&output.stdout);
        assert!(stdout.contains("Usage:"), "{flag}: {stdout:?}");
        assert!(stdout.contains("--list"), "{flag}: {stdout:?}");
        assert!(stdout.contains("--extract"), "{flag}: {stdout:?}");
    }
}

#[test]
fn unknown_option_after_a_subcommand_is_a_usage_error() {
    // A lone unknown flag is *not* a command (it falls through to the GUI); a
    // malformed option after a recognised subcommand is the usage path.
    let cases: [&[&str]; 2] = [
        &["--list", "--bogus"],
        &["--extract", "capture.rif", "--all", "--bogus"],
    ];
    for args in cases {
        let borrowed: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
        let output = cli(&borrowed);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{args:?}: stderr {}",
            text(&output.stderr)
        );
        assert!(
            !output.stderr.is_empty(),
            "{args:?}: usage errors must explain themselves"
        );
        assert!(output.stdout.is_empty(), "{args:?}: nothing goes to stdout");
    }
}

#[test]
fn list_reports_every_part_and_marks_unreadable_ones() {
    let (_dir, path) = capture_file(
        "list",
        &[
            encode_part(b"alpha", b"hello\n"),
            // No NUL separator: indexed as an unreadable placeholder.
            codec::pack(b"no-separator"),
            encode_part(b"beta", b"bye\n"),
        ],
    );

    let output = cli(&[OsStr::new("--list"), path.as_os_str()]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        text(&output.stderr)
    );

    let stdout = text(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 3, "{stdout:?}");

    let alpha: Vec<&str> = lines[0].split('\t').collect();
    assert_eq!(alpha.len(), 4, "{:?}", lines[0]);
    assert_eq!(alpha[1], "alpha");
    assert_eq!(alpha[3], "ok");
    assert!(alpha[2].parse::<u64>().expect("compressed size") > 0);

    assert!(lines[1].contains("unreadable: "), "{:?}", lines[1]);

    let beta: Vec<&str> = lines[2].split('\t').collect();
    assert_eq!(beta.len(), 4, "{:?}", lines[2]);
    assert_eq!(beta[1], "beta");
    assert_eq!(beta[3], "ok");
}

#[test]
fn extract_one_module_to_stdout_prints_exact_text() {
    let (_dir, path) = capture_file(
        "module-stdout",
        &[
            encode_part(b"alpha", b"hello\n"),
            encode_part(b"beta", b"bye\n"),
        ],
    );

    let output = cli(&[
        OsStr::new("--extract"),
        path.as_os_str(),
        OsStr::new("--module"),
        OsStr::new("alpha"),
        OsStr::new("--stdout"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        text(&output.stderr)
    );
    // A single module gets no separator header.
    assert_eq!(text(&output.stdout), "hello\n");
}

#[test]
fn extract_all_to_stdout_separates_readable_modules() {
    let (_dir, path) = capture_file(
        "all-stdout",
        &[
            encode_part(b"alpha", b"hello\n"),
            codec::pack(b"broken"),
            encode_part(b"beta", b"bye\n"),
        ],
    );

    let output = cli(&[
        OsStr::new("--extract"),
        path.as_os_str(),
        OsStr::new("--all"),
        OsStr::new("--stdout"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        text(&output.stderr)
    );
    assert_eq!(
        text(&output.stdout),
        "\n===== alpha [0] =====\nhello\n\n===== beta [0] =====\nbye\n"
    );
    assert!(
        text(&output.stderr).contains("skipping unreadable module"),
        "the unreadable part should be reported: {}",
        text(&output.stderr)
    );
}

#[test]
fn extract_to_directory_sanitises_the_label() {
    let (dir, path) = capture_file(
        "module-out",
        &[
            encode_part(b"alpha", b"ignored\n"),
            encode_part(b"/ip/firewall/filter", b"rules\n"),
        ],
    );
    let out_dir = dir.path().join("parts");

    let output = cli(&[
        OsStr::new("--extract"),
        path.as_os_str(),
        OsStr::new("--module"),
        OsStr::new("/ip/firewall/filter"),
        OsStr::new("--out"),
        out_dir.as_os_str(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(out_dir.join("ip_firewall_filter.txt")).expect("written file"),
        "rules\n"
    );
    assert!(
        text(&output.stdout).contains("ip_firewall_filter.txt"),
        "the written path should be reported: {:?}",
        text(&output.stdout)
    );
}

#[test]
fn extract_defaults_to_a_sibling_parts_directory() {
    let (dir, path) = capture_file("default-dir", &[encode_part(b"log", b"line\n")]);

    let output = cli(&[
        OsStr::new("--extract"),
        path.as_os_str(),
        OsStr::new("--module"),
        OsStr::new("log"),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("capture.parts").join("log.txt"))
            .expect("default parts file"),
        "line\n"
    );
}

#[test]
fn extract_all_disambiguates_colliding_names() {
    let (dir, path) = capture_file(
        "collision",
        &[encode_part(b"dup", b"one\n"), encode_part(b"dup", b"two\n")],
    );
    let out_dir = dir.path().join("out");

    let output = cli(&[
        OsStr::new("--extract"),
        path.as_os_str(),
        OsStr::new("--all"),
        OsStr::new("--out"),
        out_dir.as_os_str(),
    ]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr {}",
        text(&output.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(out_dir.join("dup.txt")).expect("first file"),
        "one\n"
    );
    assert_eq!(
        std::fs::read_to_string(out_dir.join("dup-2.txt")).expect("second file"),
        "two\n"
    );
}

#[test]
fn missing_capture_is_an_operational_failure() {
    let dir = TempDir::new("missing-capture");
    let path = dir.path().join("absent.rif");

    let output = cli(&[OsStr::new("--list"), path.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("cannot read"),
        "stderr {:?}",
        text(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn non_capture_file_is_an_operational_failure() {
    let dir = TempDir::new("not-a-capture");
    let path = dir.path().join("random.bin");
    std::fs::write(&path, b"not a capture").expect("write a decoy");

    let output = cli(&[OsStr::new("--list"), path.as_os_str()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("is not a capture"),
        "stderr {:?}",
        text(&output.stderr)
    );
}

#[test]
fn extract_unknown_module_is_an_operational_failure() {
    let (_dir, path) = capture_file("unknown-module", &[encode_part(b"alpha", b"hi\n")]);

    let output = cli(&[
        OsStr::new("--extract"),
        path.as_os_str(),
        OsStr::new("--module"),
        OsStr::new("nope"),
        OsStr::new("--stdout"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        text(&output.stderr).contains("no module named"),
        "stderr {:?}",
        text(&output.stderr)
    );
}

#[test]
fn list_without_a_path_is_a_usage_error() {
    let output = cli(&[OsStr::new("--list")]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("requires a capture path"),
        "stderr {:?}",
        text(&output.stderr)
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn extract_rejects_module_and_all_together() {
    let output = cli(&[
        OsStr::new("--extract"),
        OsStr::new("capture.rif"),
        OsStr::new("--all"),
        OsStr::new("--module"),
        OsStr::new("alpha"),
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        text(&output.stderr).contains("cannot be combined"),
        "stderr {:?}",
        text(&output.stderr)
    );
}

#[test]
fn bare_path_never_takes_the_cli_path() {
    // A bare path must fall through to the desktop viewer, whose event loop
    // blocks. Without a display the process aborts headless, which is
    // indistinguishable from taking the CLI path, so skip explicitly there.
    #[cfg(target_os = "linux")]
    {
        let has_x11 = std::env::var("DISPLAY")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        let has_wayland = std::env::var("WAYLAND_DISPLAY")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if !has_x11 && !has_wayland {
            eprintln!(
                "skipping bare_path_never_takes_the_cli_path: no display \
                 (DISPLAY and WAYLAND_DISPLAY are unset)"
            );
            return;
        }
    }

    let wait_ms: u64 = std::env::var("MIKROTIK_RIF_GUI_WAIT_MS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(300);
    let wait = Duration::from_millis(wait_ms);

    let (_dir, path) = capture_file("bare-path", &[encode_part(b"log", b"line\n")]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_mikrotik-rif"))
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn the viewer");

    // Liveness probe: the viewer must still be alive at the deadline. An
    // early exit means the CLI path (or a headless abort) took over.
    let deadline = Instant::now() + wait;
    let mut early_status = None;
    while Instant::now() < deadline {
        match child.try_wait().expect("poll the viewer") {
            Some(status) => {
                early_status = Some(status);
                break;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let alive_at_deadline = match child.try_wait().expect("poll the viewer") {
        Some(status) => {
            if early_status.is_none() {
                early_status = Some(status);
            }
            false
        }
        None => true,
    };
    if alive_at_deadline {
        let _ = child.kill();
    }

    let output = child.wait_with_output().expect("reap the viewer");
    let stdout = text(&output.stdout);
    let stderr = text(&output.stderr);
    let stderr_head: String = stderr.lines().take(20).collect::<Vec<_>>().join("\n");

    assert!(
        alive_at_deadline,
        "the viewer exited early (status {:?}); expected it alive after {wait_ms}ms; \
         stdout {stdout:?}; stderr head:\n{stderr_head}",
        early_status.or(Some(output.status)),
    );
    assert!(
        !stdout.contains("Usage:") && !stderr.contains("Usage:"),
        "a bare path printed CLI usage after {wait_ms}ms: stdout {stdout:?}; \
         stderr head:\n{stderr_head}"
    );
    assert!(
        !stdout.contains("headless capture reader") && !stderr.contains("headless capture reader"),
        "a bare path printed the CLI help banner after {wait_ms}ms: stdout {stdout:?}; \
         stderr head:\n{stderr_head}"
    );
}

#[test]
#[ignore = "needs a local capture"]
fn corpus_list_smoke() {
    // Opt-in smoke test over a real capture or a directory of `.rif` files:
    //
    // ```sh
    // MIKROTIK_RIF_CORPUS=/path/to/captures \
    //     cargo test --test cli -- --ignored corpus_list_smoke
    // ```
    //
    // No capture is ever committed. When the variable is unset or points at
    // something that is neither a file nor a directory, this test skips cleanly.
    let Ok(target) = std::env::var("MIKROTIK_RIF_CORPUS") else {
        eprintln!("MIKROTIK_RIF_CORPUS is not set; skipping the corpus smoke test");
        return;
    };
    let target = PathBuf::from(target);
    let location = target.display().to_string();

    let captures: Vec<PathBuf> = if target.is_file() {
        vec![target]
    } else if target.is_dir() {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&target).expect("the corpus directory must be readable") {
            let path = entry.expect("a corpus entry must be readable").path();
            if path.extension().and_then(OsStr::to_str) == Some("rif") {
                found.push(path);
            }
        }
        found
    } else {
        eprintln!("MIKROTIK_RIF_CORPUS={location} is neither a file nor a directory; skipping");
        return;
    };

    if captures.is_empty() {
        eprintln!("MIKROTIK_RIF_CORPUS={location} holds no .rif files; skipping");
        return;
    }

    for capture in &captures {
        let output = cli(&[OsStr::new("--list"), capture.as_os_str()]);
        assert!(
            output.status.success(),
            "--list failed for {}: {}",
            capture.display(),
            text(&output.stderr)
        );
        let stdout = text(&output.stdout);
        let parts = stdout.lines().filter(|line| !line.is_empty()).count();
        assert!(
            parts >= 1,
            "{} indexed no parts: {stdout:?}",
            capture.display()
        );
    }
    eprintln!("listed {} capture(s) under {location}", captures.len());
}
