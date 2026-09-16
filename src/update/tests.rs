#[cfg(target_os = "linux")]
use super::handoff::{check_appimage_target, install_appimage_file};
use super::handoff::{reverify, running_appimage_from};
use super::net::{
    FetchOutcome, HttpGet, HttpRequest, HttpResponse, download_dir, download_verified_with,
    fetch_latest_with, get_following_redirects, open_destination, resolve_redirect,
};
use super::*;

fn asset(name: &str) -> AssetInfo {
    AssetInfo {
        name: name.to_owned(),
        url: format!("https://example.com/{name}"),
    }
}

/// An asset whose URL is on an allow-listed host, so download tests reach
/// the transport instead of failing URL validation.
fn github_asset(name: &str) -> AssetInfo {
    AssetInfo {
        name: name.to_owned(),
        url: format!("https://github.com/balakar94/mikrotik-rif/releases/download/v0.3.0/{name}"),
    }
}

fn realistic_assets() -> Vec<AssetInfo> {
    vec![
        asset("mikrotik-rif_0.2.0_amd64-setup.exe"),
        asset("mikrotik-rif_0.2.0_arm64-setup.exe"),
        asset("mikrotik-rif_0.2.0_arm64.dmg"),
        asset("mikrotik-rif_0.2.0_amd64.deb"),
        asset("mikrotik-rif_0.2.0_arm64.deb"),
        asset("mikrotik-rif_0.2.0_amd64.rpm"),
        asset("mikrotik-rif_0.2.0_arm64.rpm"),
        asset("mikrotik-rif_0.2.0_amd64.AppImage"),
        asset("mikrotik-rif_0.2.0_arm64.AppImage"),
        asset("SHA256SUMS.txt"),
    ]
}

#[test]
fn endpoint_uses_the_central_coordinates() {
    let url = latest_release_url();
    assert!(url.contains(UPDATE_OWNER), "owner missing in {url}");
    assert!(url.contains(UPDATE_REPO), "repo missing in {url}");
    assert!(
        url.ends_with("/releases/latest"),
        "unexpected endpoint {url}"
    );
}

#[test]
fn repository_urls_use_the_central_coordinates() {
    let repo = repository_url();
    assert!(repo.starts_with("https://github.com/"));
    assert!(repo.contains(UPDATE_OWNER) && repo.contains(UPDATE_REPO));
    assert_eq!(releases_page_url(), format!("{repo}/releases"));
    assert!(license_url().ends_with("/blob/main/LICENSE"));
}

#[test]
fn user_agent_identifies_the_app() {
    let agent = user_agent();
    assert!(
        agent.starts_with("mikrotik-rif/"),
        "unexpected agent {agent}"
    );
    assert!(agent.contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn tag_parsing_accepts_an_optional_v_prefix() {
    assert_eq!(
        parse_tag("v1.2.3"),
        Some(semver::Version::new(1, 2, 3)),
        "v prefix"
    );
    assert_eq!(
        parse_tag("1.2.3"),
        Some(semver::Version::new(1, 2, 3)),
        "bare version"
    );
    assert!(parse_tag("not-a-version").is_none());
    assert!(parse_tag("v1.2").is_none(), "partial versions are rejected");
    assert!(parse_tag("").is_none());
}

#[test]
fn newer_stable_tags_are_updates() {
    assert!(is_update("0.1.0", "v0.2.0"));
    assert!(is_update("0.1.0", "1.0.0"));
    assert!(!is_update("0.2.0", "v0.2.0"), "equal is not newer");
    assert!(!is_update("0.3.0", "v0.2.0"), "older is not newer");
    assert!(!is_update("0.1.0", "garbage"));
    assert!(!is_update("garbage", "v9.9.9"), "bad current never updates");
}

#[test]
fn pre_release_tags_are_ignored() {
    // `releases/latest` never points at a pre-release; this guards the
    // comparison itself so a hand-crafted tag cannot sneak one through.
    assert!(!is_update("0.1.0", "v9.9.9-beta.1"));
    assert!(!is_update("0.1.0", "v0.2.0-rc.1"));
    assert!(!is_update("1.2.2", "v1.2.3-alpha"));
}

#[test]
fn running_version_parses() {
    assert!(
        semver::Version::parse(env!("CARGO_PKG_VERSION")).is_ok(),
        "CARGO_PKG_VERSION must be semantic versioning"
    );
}

#[test]
fn windows_assets_match_by_arch_marker() {
    let assets = realistic_assets();
    assert_eq!(
        select_asset(&assets, "windows", "x86_64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_amd64-setup.exe")
    );
    assert_eq!(
        select_asset(&assets, "windows", "aarch64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_arm64-setup.exe")
    );
    assert!(
        select_asset(&assets, "windows", "x86").is_none(),
        "32-bit Windows is not shipped"
    );
}

#[test]
fn macos_only_serves_apple_silicon() {
    let assets = realistic_assets();
    assert_eq!(
        select_asset(&assets, "macos", "aarch64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_arm64.dmg")
    );
    assert!(
        select_asset(&assets, "macos", "x86_64").is_none(),
        "Intel macOS is not shipped"
    );
}

#[test]
fn linux_prefers_native_packages_over_appimage() {
    let assets = realistic_assets();
    assert_eq!(
        select_asset(&assets, "linux", "x86_64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_amd64.deb"),
        "deb wins on x86_64"
    );
    assert_eq!(
        select_asset(&assets, "linux", "aarch64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_arm64.deb"),
        "deb wins on aarch64"
    );
}

#[test]
fn linux_falls_back_through_rpm_to_appimage() {
    let without_deb = vec![
        asset("mikrotik-rif_0.2.0_amd64.rpm"),
        asset("mikrotik-rif_0.2.0_amd64.AppImage"),
        asset("mikrotik-rif_0.2.0_arm64.rpm"),
        asset("mikrotik-rif_0.2.0_arm64.AppImage"),
    ];
    assert_eq!(
        select_asset(&without_deb, "linux", "x86_64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_amd64.rpm")
    );
    let portable_only = vec![
        asset("mikrotik-rif_0.2.0_amd64.AppImage"),
        asset("mikrotik-rif_0.2.0_arm64.AppImage"),
    ];
    assert_eq!(
        select_asset(&portable_only, "linux", "aarch64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_arm64.AppImage")
    );
}

#[test]
fn linux_never_crosses_architectures() {
    let x64_only = vec![
        asset("mikrotik-rif_0.2.0_amd64.deb"),
        asset("mikrotik-rif_0.2.0_amd64.rpm"),
        asset("mikrotik-rif_0.2.0_amd64.AppImage"),
    ];
    assert!(
        select_asset(&x64_only, "linux", "aarch64").is_none(),
        "amd64 assets must not serve arm64"
    );
    assert!(
        select_asset(&[], "linux", "x86_64").is_none(),
        "empty release has no asset"
    );
}

#[test]
fn unknown_platforms_fall_back_to_the_release_page() {
    let assets = realistic_assets();
    assert!(select_asset(&assets, "freebsd", "x86_64").is_none());
    assert!(select_asset(&assets, "windows", "riscv64").is_none());
}

#[test]
fn checksums_url_finds_the_attached_sums() {
    let release = ReleaseInfo {
        tag: "v0.2.0".to_owned(),
        name: None,
        body: None,
        page_url: "https://example.com/release".to_owned(),
        assets: realistic_assets(),
    };
    let url = checksums_url(&release).expect("SHA256SUMS.txt is attached");
    assert!(url.ends_with("SHA256SUMS.txt"), "unexpected url {url}");
    let bare = ReleaseInfo {
        assets: vec![asset("mikrotik-rif_0.2.0_amd64.deb")],
        ..release
    };
    assert!(checksums_url(&bare).is_none());
}

#[test]
fn checksum_lookup_parses_sums_lines() {
    let sums = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  mikrotik-rif_0.2.0_amd64.deb\n\
                    9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08 *other.deb\n";
    assert_eq!(
        find_checksum(sums, "mikrotik-rif_0.2.0_amd64.deb"),
        Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_owned())
    );
    assert_eq!(
        find_checksum(sums, "other.deb"),
        Some("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08".to_owned()),
        "binary marker tolerated"
    );
    assert!(find_checksum(sums, "missing.deb").is_none());
    assert!(find_checksum("not a sums line\n", "a").is_none());
    assert!(
        find_checksum("xyz  a.deb\n", "a.deb").is_none(),
        "short hash rejected"
    );
}

#[test]
fn sha256_known_vector_without_network() {
    // Well-known SHA-256 of "abc"; local data only.
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let digest = hex_encode(Sha256::digest(b"abc").as_slice());
    assert_eq!(digest, expected);
    assert!(digests_match(&digest, &expected.to_ascii_uppercase()));
    assert!(!digests_match(&digest, "00"));
    assert_eq!(hex_encode(&[]), "");
    assert_eq!(hex_encode(&[0x00, 0x0f, 0xff]), "000fff");
}

/// Scratch file unique to this test process and counter.
fn scratch_file(contents: &[u8]) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mikrotik-rif-update-test-{}-{id}.bin",
        std::process::id()
    ));
    std::fs::write(&path, contents).expect("scratch file is writable");
    path
}

/// Unique scratch path that does not exist yet.
fn scratch_path() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "mikrotik-rif-update-test-{}-{id}.link",
        std::process::id()
    ))
}

#[test]
fn verified_files_are_kept_and_bad_ones_deleted() {
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let sums = format!("{expected}  installer.bin\n");

    let kept = scratch_file(b"abc");
    match verify_against_sums(&sums, "installer.bin", &kept) {
        DownloadOutcome::Done { path, expected_hex } => {
            assert_eq!(path, kept);
            assert_eq!(expected_hex, expected);
        }
        other => panic!("verified file must be kept, got {other:?}"),
    }
    assert!(kept.is_file(), "verified file stays on disk");
    std::fs::remove_file(&kept).expect("cleanup");

    let tampered = scratch_file(b"abd");
    match verify_against_sums(&sums, "installer.bin", &tampered) {
        DownloadOutcome::Failed(_) => {}
        other => panic!("tampered file must fail, got {other:?}"),
    }
    assert!(
        !tampered.exists(),
        "tampered file is deleted, never executed"
    );

    let unknown = scratch_file(b"abc");
    match verify_against_sums(&sums, "other.bin", &unknown) {
        DownloadOutcome::Failed(_) => {}
        other => panic!("missing entry must fail, got {other:?}"),
    }
    assert!(!unknown.exists(), "unverifiable file is deleted");
}

#[test]
fn auto_check_follows_the_toggle() {
    assert!(should_auto_check(true), "on by default");
    assert!(!should_auto_check(false), "opt-out wins");
}

#[test]
fn appimage_env_reports_the_running_image() {
    use std::ffi::OsString;
    assert_eq!(
        running_appimage_from(Some(OsString::from("/opt/app.AppImage"))),
        Some(PathBuf::from("/opt/app.AppImage"))
    );
    assert_eq!(running_appimage_from(None), None);
    assert_eq!(running_appimage_from(Some(OsString::new())), None);
}

#[test]
fn appimage_lookup_finds_the_portable_asset() {
    let assets = realistic_assets();
    assert_eq!(
        find_appimage(&assets, "_amd64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_amd64.AppImage")
    );
    assert_eq!(
        find_appimage(&assets, "_arm64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_arm64.AppImage")
    );
    assert!(find_appimage(&assets, "riscv64").is_none());
    // The preference is not a filter: `select_asset` still prefers the
    // native package; only `select_asset_for_current` promotes the image
    // for a running AppImage.
    assert_eq!(
        select_asset(&assets, "linux", "x86_64").map(|found| found.name.as_str()),
        Some("mikrotik-rif_0.2.0_amd64.deb")
    );
}

/// Unique scratch directory per test (tests share one process, so the
/// name combines the pid with an atomic counter).
#[cfg(target_os = "linux")]
fn scratch_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "mikrotik-rif-update-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
#[cfg(target_os = "linux")]
fn self_replace_ignores_non_appimage_files() {
    // No environment involved: a native package never self-replaces.
    assert!(maybe_self_replace(Path::new("mikrotik-rif_0.2.0_amd64.deb"), "00").is_none());
    assert!(maybe_self_replace(Path::new("release-notes.txt"), "00").is_none());
}

#[test]
#[cfg(target_os = "linux")]
fn appimage_target_must_be_a_regular_file() {
    let dir = scratch_dir();
    let file = dir.join("mikrotik-rif.AppImage");
    std::fs::write(&file, b"image").expect("write file");
    assert!(check_appimage_target(&file).is_ok());
    assert!(
        check_appimage_target(&dir).is_err(),
        "a directory is not a valid AppImage target"
    );
    assert!(
        check_appimage_target(&dir.join("missing.AppImage")).is_err(),
        "a missing path is not a valid AppImage target"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
#[cfg(target_os = "linux")]
fn appimage_install_replaces_atomically() {
    let dir = scratch_dir();
    let current = dir.join("mikrotik-rif.AppImage");
    let new_file = dir.join("downloaded.AppImage");
    std::fs::write(&current, b"old-bytes").expect("write current");
    std::fs::write(&new_file, b"new-bytes").expect("write new");

    install_appimage_file(&current, &new_file).expect("replace");
    assert_eq!(std::fs::read(&current).expect("read back"), b"new-bytes");
    assert!(
        !dir.join("mikrotik-rif.AppImage.update").exists(),
        "staging file is gone after the rename"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&current)
            .expect("metadata")
            .permissions()
            .mode();
        assert_ne!(mode & 0o111, 0, "replaced image stays executable");
    }

    // A missing download never touches the running image.
    let missing = dir.join("absent.AppImage");
    assert!(install_appimage_file(&current, &missing).is_err());
    assert_eq!(std::fs::read(&current).expect("read back"), b"new-bytes");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_https_github_urls_are_allowed() {
    for url in [
        "https://github.com/balakar94/mikrotik-rif/releases/download/v1/x.deb",
        "https://api.github.com/repos/balakar94/mikrotik-rif/releases/latest",
        "https://objects.githubusercontent.com/github-production-release-asset/x",
        "https://release-assets.githubusercontent.com/github-production-release-asset/x",
    ] {
        assert!(check_url(url).is_ok(), "{url} must be accepted");
    }
    for url in [
        "http://github.com/x",
        "file:///etc/passwd",
        "evil://payload",
        "https://evil.example.com/x",
        "https://github.com.evil.example.com/x",
        "https://raw.githubusercontent.com/x",
        "not a url",
        "",
    ] {
        assert!(
            matches!(check_url(url), Err(UpdateError::InvalidUrl(_))),
            "{url} must be refused"
        );
    }
}

#[test]
fn asset_dest_strips_path_traversal() {
    let traversal = asset_dest("../../evil.exe");
    assert_eq!(
        traversal.file_name().and_then(|name| name.to_str()),
        Some("evil.exe"),
        "directory components are stripped"
    );
    assert!(
        traversal.starts_with(download_dir()),
        "destination stays inside the download directory"
    );
    let empty = asset_dest("");
    assert_eq!(
        empty.file_name().and_then(|name| name.to_str()),
        Some("mikrotik-rif-update.bin"),
        "a nameless asset gets a safe fallback"
    );
}

#[test]
fn oversized_release_payloads_are_rejected() {
    let mut release = ReleaseInfo {
        tag: "v0.2.0".to_owned(),
        name: None,
        body: None,
        page_url: "https://github.com/balakar94/mikrotik-rif/releases/tag/v0.2.0".to_owned(),
        assets: Vec::new(),
    };
    assert!(
        validate_release(&release).is_ok(),
        "a normal release passes"
    );

    release.assets = (0..=MAX_ASSETS)
        .map(|index| AssetInfo {
            name: format!("asset-{index}.bin"),
            url: format!("https://github.com/asset-{index}"),
        })
        .collect();
    assert!(
        matches!(validate_release(&release), Err(UpdateError::Response(_))),
        "too many assets is rejected"
    );

    release.assets = vec![asset("ok.bin")];
    release.body = Some("x".repeat(MAX_NOTES_BYTES + 1));
    assert!(
        matches!(validate_release(&release), Err(UpdateError::Response(_))),
        "oversized notes are rejected"
    );
    release.body = None;
    release.tag = "v".repeat(MAX_FIELD_BYTES + 1);
    assert!(
        matches!(validate_release(&release), Err(UpdateError::Response(_))),
        "oversized identifier fields are rejected"
    );
}

#[test]
fn reverify_refuses_a_file_swapped_after_verification() {
    let good = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let file = scratch_file(b"abc");
    assert!(reverify(&file, good).is_ok(), "matching digest passes");

    // A local process swaps the verified file before it is used.
    std::fs::write(&file, b"tampered").expect("overwrite");
    assert!(
        matches!(reverify(&file, good), Err(UpdateError::HashMismatch(_))),
        "a swapped file is refused"
    );
    assert!(!file.exists(), "the swapped file is deleted");
}

#[test]
fn release_asset_redirect_targets_are_allowed() {
    // Both the historical and the current GitHub object-store hosts must
    // pass, or every installer download fails at the redirect hop.
    for url in [
        "https://objects.githubusercontent.com/github-production-release-asset/1/2",
        "https://release-assets.githubusercontent.com/github-production-release-asset/1/2",
    ] {
        assert!(check_url(url).is_ok(), "{url} must be accepted");
    }
}

#[test]
fn lookalike_asset_hosts_are_refused() {
    for url in [
        "https://release-assets.githubusercontent.com.evil.example/x",
        "https://xrelease-assets.githubusercontent.com/x",
        "https://github.com@release-assets.githubusercontent.com.evil.example/x",
    ] {
        assert!(check_url(url).is_err(), "{url} must be refused");
    }
}

#[test]
fn current_build_workflow_names_match_the_selector() {
    // These are the exact artifacts `build.yml` produces after its
    // "Normalise release asset names" step. If the workflow renames any of
    // them, this test (and the module's contract table) must change too.
    let assets = vec![
        asset("mikrotik-rif_0.3.0_amd64-setup.exe"),
        asset("mikrotik-rif_0.3.0_arm64-setup.exe"),
        asset("mikrotik-rif_0.3.0_arm64.dmg"),
        asset("mikrotik-rif_0.3.0_amd64.deb"),
        asset("mikrotik-rif_0.3.0_arm64.deb"),
        asset("mikrotik-rif_0.3.0_amd64.rpm"),
        asset("mikrotik-rif_0.3.0_arm64.rpm"),
        asset("mikrotik-rif_0.3.0_amd64.AppImage"),
        asset("mikrotik-rif_0.3.0_arm64.AppImage"),
        asset("SHA256SUMS.txt"),
    ];
    for (os, arch, expected) in [
        ("windows", "x86_64", "mikrotik-rif_0.3.0_amd64-setup.exe"),
        ("windows", "aarch64", "mikrotik-rif_0.3.0_arm64-setup.exe"),
        ("macos", "aarch64", "mikrotik-rif_0.3.0_arm64.dmg"),
        ("linux", "x86_64", "mikrotik-rif_0.3.0_amd64.deb"),
        ("linux", "aarch64", "mikrotik-rif_0.3.0_arm64.deb"),
    ] {
        assert_eq!(
            select_asset(&assets, os, arch).map(|found| found.name.as_str()),
            Some(expected),
            "{os}/{arch}"
        );
    }
    let release = ReleaseInfo {
        tag: "v0.3.0".to_owned(),
        name: None,
        body: None,
        page_url: "https://github.com/balakar94/mikrotik-rif/releases/tag/v0.3.0".to_owned(),
        assets,
    };
    assert_eq!(
        checksums_url(&release).as_deref(),
        Some("https://example.com/SHA256SUMS.txt")
    );
}

#[test]
fn checksum_entry_must_match_the_exact_asset_name() {
    // v0.2.0 shipped a DMG whose local name had spaces; GitHub published it
    // with dots while SHA256SUMS.txt kept the local name, so the lookup
    // missed. Names must now match byte-for-byte.
    let sums = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad  \
                    MikroTik RIF Viewer_0.2.0_aarch64.dmg\n";
    assert!(find_checksum(sums, "MikroTik RIF Viewer_0.2.0_aarch64.dmg").is_some());
    assert!(
        find_checksum(sums, "MikroTik.RIF.Viewer_0.2.0_aarch64.dmg").is_none(),
        "a rewritten file name must not match"
    );
}

#[test]
fn open_destination_creates_a_fresh_file() {
    let path = scratch_path();
    let file = open_destination(&path).expect("create");
    drop(file);
    assert!(path.is_file());
    let _ = std::fs::remove_file(&path);
}

#[cfg(unix)]
#[test]
fn open_destination_does_not_follow_a_planted_symlink() {
    use std::os::unix::fs::symlink;

    let victim = scratch_file(b"victim-bytes");
    let path = scratch_path();
    symlink(&victim, &path).expect("plant symlink");

    let file = open_destination(&path).expect("create");
    drop(file);

    assert_eq!(
        std::fs::read(&victim).expect("victim intact"),
        b"victim-bytes",
        "the symlink target must not be truncated"
    );
    assert!(
        std::fs::symlink_metadata(&path)
            .expect("metadata")
            .file_type()
            .is_file(),
        "the destination is a fresh regular file"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&victim);
}

// ── Network behaviour, exercised through a fake transport ───────────────

/// One scripted response for [`FakeTransport`].
struct FakeResponse {
    status: u16,
    location: Option<&'static str>,
    content_length: Option<u64>,
    etag: Option<&'static str>,
    body: Vec<u8>,
}

impl FakeResponse {
    fn ok(body: &[u8]) -> Self {
        Self {
            status: 200,
            location: None,
            content_length: u64::try_from(body.len()).ok(),
            etag: None,
            body: body.to_vec(),
        }
    }

    fn redirect(location: &'static str) -> Self {
        Self {
            status: 302,
            location: Some(location),
            content_length: None,
            etag: None,
            body: Vec::new(),
        }
    }
}

/// Transport that hands out scripted responses and records every request.
#[derive(Default)]
struct FakeTransport {
    responses: std::sync::Mutex<std::collections::VecDeque<FakeResponse>>,
    requests: std::sync::Mutex<Vec<String>>,
    etags: std::sync::Mutex<Vec<Option<String>>>,
}

impl FakeTransport {
    fn new(responses: Vec<FakeResponse>) -> Self {
        Self {
            responses: std::sync::Mutex::new(responses.into()),
            ..Self::default()
        }
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().expect("requests").clone()
    }

    fn etags(&self) -> Vec<Option<String>> {
        self.etags.lock().expect("etags").clone()
    }
}

impl HttpGet for FakeTransport {
    fn get(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, UpdateError> {
        self.requests
            .lock()
            .expect("requests")
            .push(request.url.to_owned());
        self.etags
            .lock()
            .expect("etags")
            .push(request.if_none_match.map(ToOwned::to_owned));
        let response = self
            .responses
            .lock()
            .expect("responses")
            .pop_front()
            .ok_or_else(|| UpdateError::Network("fake transport ran out".to_owned()))?;
        Ok(HttpResponse::new(
            response.status,
            response.location.map(ToOwned::to_owned),
            response.content_length,
            response.etag.map(ToOwned::to_owned),
            Box::new(std::io::Cursor::new(response.body)),
        ))
    }
}

#[test]
fn relative_redirects_resolve_against_the_origin() {
    assert_eq!(
        resolve_redirect("https://github.com/a/b", "/c").expect("absolute path"),
        "https://github.com/c"
    );
    assert_eq!(
        resolve_redirect("https://github.com/a/b", "c").expect("relative path"),
        "https://github.com/a/c"
    );
    assert_eq!(
        resolve_redirect("https://github.com/a/b", "//x.example/c").expect("scheme-relative"),
        "https://x.example/c"
    );
    assert_eq!(
        resolve_redirect("https://github.com/a/b", "https://y.example/c").expect("absolute"),
        "https://y.example/c"
    );
    assert!(resolve_redirect("https://github.com/", "").is_err());
}

#[test]
fn redirects_are_followed_and_each_hop_is_validated() {
    let transport = FakeTransport::new(vec![
        FakeResponse::redirect("https://release-assets.githubusercontent.com/asset/1"),
        FakeResponse::ok(b"payload"),
    ]);
    let response = get_following_redirects(&transport, "https://github.com/x", "", None)
        .expect("allowed redirect chain");
    assert_eq!(response.status(), 200);
    assert_eq!(transport.requests().len(), 2);
    let mut body = String::new();
    response
        .into_body()
        .read_to_string(&mut body)
        .expect("body");
    assert_eq!(body, "payload");
}

#[test]
fn redirect_to_a_disallowed_host_is_refused_before_it_is_requested() {
    let transport = FakeTransport::new(vec![FakeResponse::redirect("https://evil.example/x")]);
    let Err(error) = get_following_redirects(&transport, "https://github.com/x", "", None) else {
        panic!("off-list host must be refused");
    };
    assert!(matches!(error, UpdateError::InvalidUrl(_)));
    assert_eq!(
        transport.requests().len(),
        1,
        "the disallowed host is never contacted"
    );
}

#[test]
fn relative_redirect_is_requested_against_the_origin() {
    let transport = FakeTransport::new(vec![
        FakeResponse::redirect("/next"),
        FakeResponse::ok(b"ok"),
    ]);
    get_following_redirects(&transport, "https://github.com/a/b", "", None).expect("resolves");
    assert_eq!(transport.requests()[1], "https://github.com/next");
}

#[test]
fn redirect_loops_are_capped() {
    let responses = (0..=MAX_REDIRECTS + 2)
        .map(|_| FakeResponse::redirect("https://github.com/loop"))
        .collect();
    let transport = FakeTransport::new(responses);
    let Err(error) = get_following_redirects(&transport, "https://github.com/start", "", None)
    else {
        panic!("loop must be capped");
    };
    assert!(matches!(error, UpdateError::InvalidUrl(_)));
}

#[test]
fn not_modified_carries_no_body_and_keeps_the_conditional_header() {
    let transport = FakeTransport::new(vec![FakeResponse {
        status: 304,
        location: None,
        content_length: None,
        etag: None,
        body: Vec::new(),
    }]);
    assert!(matches!(
        fetch_latest_with(&transport, Some("\"etag-1\"")).expect("304 is not an error"),
        FetchOutcome::NotModified
    ));
    assert_eq!(transport.etags()[0].as_deref(), Some("\"etag-1\""));
}

#[test]
fn metadata_body_and_etag_are_captured() {
    let body = br#"{"tag_name":"v9.9.9","html_url":"https://github.com/balakar94/mikrotik-rif/releases/tag/v9.9.9","assets":[]}"#;
    let transport = FakeTransport::new(vec![FakeResponse {
        status: 200,
        location: None,
        content_length: None,
        etag: Some("\"abc\""),
        body: body.to_vec(),
    }]);
    match fetch_latest_with(&transport, None).expect("parses") {
        FetchOutcome::Modified { release, etag } => {
            assert_eq!(release.tag, "v9.9.9");
            assert_eq!(etag.as_deref(), Some("\"abc\""));
        }
        FetchOutcome::NotModified => panic!("expected a release body"),
    }
}

#[test]
fn download_is_streamed_to_disk_and_verified() {
    let payload = b"installer-bytes";
    let digest = hex_encode(Sha256::digest(payload).as_slice());
    let sums = format!("{digest}  mikrotik-rif_0.3.0_amd64.deb\n");
    let transport = FakeTransport::new(vec![
        FakeResponse {
            status: 200,
            location: None,
            content_length: u64::try_from(payload.len()).ok(),
            etag: None,
            body: payload.to_vec(),
        },
        FakeResponse::ok(sums.as_bytes()),
    ]);
    let asset = github_asset("mikrotik-rif_0.3.0_amd64.deb");
    let dest = scratch_path();
    match download_verified_with(
        &transport,
        &asset,
        "https://github.com/sums",
        None,
        "",
        &dest,
        &|_, _| {},
    ) {
        DownloadOutcome::Done { path, .. } => assert_eq!(path, dest),
        other => panic!("expected Done, got {other:?}"),
    }
    assert_eq!(std::fs::read(&dest).expect("read back"), payload);
    let _ = std::fs::remove_file(&dest);
}

#[test]
fn download_refuses_an_oversized_content_length() {
    let transport = FakeTransport::new(vec![FakeResponse {
        status: 200,
        location: None,
        content_length: Some(MAX_INSTALLER_BYTES + 1),
        etag: None,
        body: vec![0_u8; 8],
    }]);
    let asset = github_asset("mikrotik-rif_0.3.0_amd64.deb");
    let dest = scratch_path();
    match download_verified_with(
        &transport,
        &asset,
        "https://github.com/sums",
        None,
        "",
        &dest,
        &|_, _| {},
    ) {
        DownloadOutcome::Failed(_) => {}
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert!(!dest.exists(), "nothing is written or left behind");
}

#[test]
fn a_configured_key_requires_a_signature() {
    let payload = b"x";
    let digest = hex_encode(Sha256::digest(payload).as_slice());
    let sums = format!("{digest}  mikrotik-rif_0.3.0_amd64.deb\n");
    let transport = FakeTransport::new(vec![
        FakeResponse::ok(payload),
        FakeResponse::ok(sums.as_bytes()),
    ]);
    let asset = github_asset("mikrotik-rif_0.3.0_amd64.deb");
    let dest = scratch_path();
    let result = download_verified_with(
        &transport,
        &asset,
        "https://github.com/sums",
        None,
        "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3",
        &dest,
        &|_, _| {},
    );
    match result {
        DownloadOutcome::Failed(reason) => {
            assert!(reason.contains(SIGNATURE_FILE_NAME), "got {reason}");
        }
        other => panic!("expected MissingSignature, got {other:?}"),
    }
    assert!(!dest.exists(), "an unsigned download is deleted");
}

#[test]
fn minisign_signatures_verify_and_tampering_is_refused() {
    // Fixture from the `minisign-verify` crate's own test suite.
    let public_key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    let signature = "untrusted comment: signature from minisign secret key\n\
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n\
trusted comment: timestamp:1556193335\tfile:test\n\
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";
    verify_minisign(b"test", signature, public_key).expect("valid signature");
    assert!(matches!(
        verify_minisign(b"tampered", signature, public_key),
        Err(UpdateError::Response(_))
    ));
    assert!(matches!(
        verify_minisign(b"test", signature, "not-a-key"),
        Err(UpdateError::Response(_))
    ));
}

// ── Wave2B hardening ────────────────────────────────────────────────────

#[test]
fn etag_sanitization_rejects_unsafe_values() {
    // Valid weak and strong validators survive, trimmed.
    assert_eq!(
        sanitize_etag("W/\"abc123\"").as_deref(),
        Some("W/\"abc123\"")
    );
    assert_eq!(sanitize_etag("\"abc\"").as_deref(), Some("\"abc\""));
    assert_eq!(
        sanitize_etag("  \"abc\"  ").as_deref(),
        Some("\"abc\""),
        "surrounding spaces are trimmed"
    );
    // CRLF injection is refused.
    assert!(sanitize_etag("evil\r\nInjected: 1").is_none());
    assert!(sanitize_etag("a\nb").is_none());
    assert!(sanitize_etag("a\rb").is_none());
    // Other controls, DEL and non-ASCII are refused.
    assert!(sanitize_etag("a\x00b").is_none(), "NUL refused");
    assert!(sanitize_etag("a\tb").is_none(), "TAB refused");
    assert!(sanitize_etag("a\x1fb").is_none(), "control refused");
    assert!(sanitize_etag("a\x7fb").is_none(), "DEL refused");
    assert!(sanitize_etag("é").is_none(), "non-ASCII refused");
    assert!(
        sanitize_etag("\"aé\"").is_none(),
        "non-ASCII inside refused"
    );
    // Length cap: 2048 passes, 2049 does not.
    assert!(sanitize_etag(&"a".repeat(2048)).is_some(), "boundary kept");
    assert!(
        sanitize_etag(&"a".repeat(2049)).is_none(),
        "oversized refused"
    );
}

#[test]
fn fetch_sanitizes_network_and_storage_etags() {
    // A corrupt network ETag never reaches storage: it becomes `None`.
    let body = br#"{"tag_name":"v9.9.9","html_url":"https://github.com/balakar94/mikrotik-rif/releases/tag/v9.9.9","assets":[]}"#;
    let transport = FakeTransport::new(vec![FakeResponse {
        status: 200,
        location: None,
        content_length: None,
        etag: Some("bad\r\nInjected: 1"),
        body: body.to_vec(),
    }]);
    match fetch_latest_with(&transport, None).expect("parses") {
        FetchOutcome::Modified { etag, .. } => {
            assert!(etag.is_none(), "CRLF ETag must be dropped, got {etag:?}");
        }
        FetchOutcome::NotModified => panic!("expected a release body"),
    }
    // An oversized network ETag is dropped the same way.
    let oversized: &'static str = Box::leak("a".repeat(2049).into_boxed_str());
    let transport = FakeTransport::new(vec![FakeResponse {
        status: 200,
        location: None,
        content_length: None,
        etag: Some(oversized),
        body: body.to_vec(),
    }]);
    match fetch_latest_with(&transport, None).expect("parses") {
        FetchOutcome::Modified { etag, .. } => {
            assert!(etag.is_none(), "oversized ETag must be dropped");
        }
        FetchOutcome::NotModified => panic!("expected a release body"),
    }
    // A corrupt stored ETag is never echoed as `If-None-Match`.
    let transport = FakeTransport::new(vec![FakeResponse {
        status: 304,
        location: None,
        content_length: None,
        etag: None,
        body: Vec::new(),
    }]);
    assert!(matches!(
        fetch_latest_with(&transport, Some("bad\r\nx")).expect("304 is not an error"),
        FetchOutcome::NotModified
    ));
    assert_eq!(
        transport.etags()[0],
        None,
        "corrupt conditional must not be sent"
    );
}

#[test]
fn tampered_sums_are_refused_before_checksum_lookup() {
    // Fixed valid vector (`minisign-verify` suite): `b"test"` under this key.
    let public_key = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
    let signature = "untrusted comment: signature from minisign secret key\n\
RUQf6LRCGA9i559r3g7V1qNyJDApGip8MfqcadIgT9CuhV3EMhHoN1mGTkUidF/z7SrlQgXdy8ofjb7bNJJylDOocrCo8KLzZwo=\n\
trusted comment: timestamp:1556193335\tfile:test\n\
y/rUw2y8/hOUYjZU71eHp/Wo1KZ40fGy2VJEDl34XMJM+TX48Ss/17u3IvIfbVR1FkZZSNCisQbuQY+bHwhEBg==";
    let payload = b"installer-bytes";
    let asset = github_asset("mikrotik-rif_0.3.0_amd64.deb");

    // Untouched sums pass the signature layer (then fail on the missing
    // checksum entry, proving verification succeeded).
    let transport = FakeTransport::new(vec![
        FakeResponse::ok(payload),
        FakeResponse::ok(b"test"),
        FakeResponse::ok(signature.as_bytes()),
    ]);
    let dest = scratch_path();
    match download_verified_with(
        &transport,
        &asset,
        "https://github.com/sums",
        Some("https://github.com/sums.minisig"),
        public_key,
        &dest,
        &|_, _| {},
    ) {
        DownloadOutcome::Failed(reason) => {
            assert!(
                reason.contains("no checksum entry"),
                "valid signature must reach checksum lookup, got {reason}"
            );
        }
        other => panic!("expected MissingChecksum, got {other:?}"),
    }
    assert!(!dest.exists(), "unverifiable download is deleted");

    // The same signature over tampered sums is refused at the signature
    // layer and the installer is deleted without execution.
    let transport = FakeTransport::new(vec![
        FakeResponse::ok(payload),
        FakeResponse::ok(b"tampered"),
        FakeResponse::ok(signature.as_bytes()),
    ]);
    let dest = scratch_path();
    match download_verified_with(
        &transport,
        &asset,
        "https://github.com/sums",
        Some("https://github.com/sums.minisig"),
        public_key,
        &dest,
        &|_, _| {},
    ) {
        DownloadOutcome::Failed(reason) => {
            assert!(
                reason.contains("signature check failed"),
                "tampered sums must fail verification, got {reason}"
            );
        }
        other => panic!("expected signature refusal, got {other:?}"),
    }
    assert!(!dest.exists(), "tampered download is deleted");
}

#[test]
fn download_to_a_directory_fails_cleanly() {
    // A pre-existing directory at `dest` must become a clean `Io` error, not
    // a truncation or a panic; the directory itself survives.
    let dir = std::env::temp_dir().join(format!(
        "mikrotik-rif-update-test-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let transport = FakeTransport::new(vec![FakeResponse::ok(b"installer-bytes")]);
    let asset = github_asset("mikrotik-rif_0.3.0_amd64.deb");
    match download_verified_with(
        &transport,
        &asset,
        "https://github.com/sums",
        None,
        "",
        &dir,
        &|_, _| {},
    ) {
        DownloadOutcome::Failed(reason) => {
            assert!(
                reason.starts_with("update failed:"),
                "directory dest must be an Io error, got {reason}"
            );
        }
        other => panic!("expected Io refusal, got {other:?}"),
    }
    assert!(dir.is_dir(), "the directory is left untouched");
    let _ = std::fs::remove_dir(&dir);
}

/// Endless zero stream without allocating the bytes up front.
struct LongReader {
    remaining: u64,
}

impl std::io::Read for LongReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 {
            return Ok(0);
        }
        let remaining = usize::try_from(self.remaining).unwrap_or(usize::MAX);
        let take = buf.len().min(remaining);
        buf[..take].fill(0);
        self.remaining = self
            .remaining
            .saturating_sub(u64::try_from(take).unwrap_or(u64::MAX));
        Ok(take)
    }
}

/// Transport that lies about `Content-Length` (small) while streaming a body
/// larger than [`MAX_INSTALLER_BYTES`].
struct LyingTransport;

impl HttpGet for LyingTransport {
    fn get(&self, request: &HttpRequest<'_>) -> Result<HttpResponse, UpdateError> {
        check_url(request.url)?;
        Ok(HttpResponse::new(
            200,
            None,
            Some(4),
            None,
            Box::new(LongReader {
                remaining: MAX_INSTALLER_BYTES + 1,
            }),
        ))
    }
}

#[test]
fn lying_content_length_is_cut_mid_stream_and_cleaned_up() {
    let asset = github_asset("mikrotik-rif_0.3.0_amd64.deb");
    let dest = scratch_path();
    match download_verified_with(
        &LyingTransport,
        &asset,
        "https://github.com/sums",
        None,
        "",
        &dest,
        &|_, _| {},
    ) {
        DownloadOutcome::Failed(reason) => {
            assert!(
                reason.contains("larger than"),
                "oversized stream must be refused, got {reason}"
            );
        }
        other => panic!("expected size refusal, got {other:?}"),
    }
    assert!(
        !dest.exists(),
        "the partial file is removed, never left as valid"
    );
}
