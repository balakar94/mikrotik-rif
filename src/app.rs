//! Desktop shell: welcome, home, opening animation and the module workspace.
//!
//! The interface thread never parses or inflates anything itself. It renders the
//! current stage and delegates all heavy work to [`Worker`]. Custom-painted
//! screens read the [`Palette`] that matches the operating system theme.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

mod settings;
mod update_ui;
mod workspace;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Align, Color32, Layout, RichText, ThemePreference};
use fluent_bundle::FluentArgs;

use crate::i18n::I18n;
use crate::parser::{
    Capture, CaptureLimits, Part, PartText, compute_view, filter_parts, next_match,
};
use crate::update::{self, ReleaseInfo};

use crate::splash::{self, ScanView};
use crate::theme::{self, Palette};
use crate::worker::{Event, Worker};

use self::settings::SettingsTab;
use self::update_ui::{CheckState, DownloadState};
use self::workspace::{
    FindState, find_case_insensitive, human_size_u64, sanitize, to_number, units,
};
use crate::worker::MAX_CAPTURE_BYTES;

/// Minimum seconds the opening animation stays on screen.
const SCAN_MIN_SECONDS: f64 = 2.4;
/// Extra seconds the animation holds after indexing, so it never cuts abruptly.
const SCAN_HOLD_SECONDS: f64 = 1.1;
/// Seconds the stage-fade overlay takes to disappear.
const FADE_SECONDS: f32 = 0.35;

/// Which screen the shell is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// First-run welcome with the Start action.
    Welcome,
    /// Where the user chooses how to get a capture.
    Home,
    /// Opening animation while the capture is read and indexed.
    Scanning,
    /// Module list and text output.
    Workspace,
}

/// eframe storage key for the automatic update check toggle.
const UPDATE_AUTO_KEY: &str = "update.auto_check";
/// eframe storage key for the last completed update check (Unix seconds).
const UPDATE_LAST_KEY: &str = "update.last_check";
/// eframe storage key for the release tag the user chose to skip.
const UPDATE_SKIPPED_KEY: &str = "update.skipped_tag";
/// eframe storage key for the last release-metadata ETag (conditional checks).
const UPDATE_ETAG_KEY: &str = "update.etag";
/// eframe storage key for the theme preference (`system` / `light` / `dark`).
const THEME_KEY: &str = "app.theme";
/// eframe storage key for the selected language tag (empty means system).
const LANGUAGE_KEY: &str = "app.language";

/// A transient message shown in the footer.
struct Notice {
    message: String,
    is_error: bool,
}

/// Expanded module text plus the line index the reading surface needs.
///
/// Cached per module index so returning to an already-expanded module is an
/// `Arc` clone and never re-indexes the text. [`PartText`] is shared, so the
/// worker can hand back either an owned text or an [`Arc`] without the cache
/// depending on which one the worker contract uses.
struct BodyView {
    /// Expanded text.
    text: Arc<PartText>,
    /// Byte offset at which each line starts.
    line_starts: Vec<usize>,
    /// Longest line in characters, ignoring line endings.
    max_line_chars: usize,
}

/// Number of expanded modules kept in the view cache.
const VIEW_CACHE_ENTRIES: usize = 8;

/// Byte budget for cached expanded module text.
const VIEW_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// Display state of the structural-notes banner for the open capture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotesState {
    /// One-line warning, detail list hidden.
    Collapsed,
    /// Detail list visible.
    Expanded,
    /// Hidden until another capture is opened.
    Dismissed,
}

/// State of a search across every module of the open capture.
///
/// Modules are expanded one at a time through the worker, so a large capture
/// is scanned without blocking the interface and the scan can be cancelled at
/// any point between modules. The partial hit set is only annotated once the
/// scan has finished, so an incomplete list never masquerades as a full one.
struct GlobalSearch {
    /// Query used for the substring test.
    needle: String,
    /// Readable module indices not yet scanned.
    pending: VecDeque<usize>,
    /// Module index whose expansion the search is waiting on.
    in_flight: Option<usize>,
    /// Module indices whose expanded text contains the query.
    hits: HashSet<usize>,
    /// Modules scanned so far.
    scanned: usize,
    /// Readable modules to scan in total.
    total: usize,
    /// Whether the scan finished (or never started).
    done: bool,
}

impl GlobalSearch {
    /// An idle search: no query, no hits, nothing to scan.
    fn idle() -> Self {
        Self {
            needle: String::new(),
            pending: VecDeque::new(),
            in_flight: None,
            hits: HashSet::new(),
            scanned: 0,
            total: 0,
            done: true,
        }
    }

    /// Whether the scan is still expanding modules.
    const fn is_scanning(&self) -> bool {
        !self.done
    }
}

/// Why a capture path was rejected before indexing.
///
/// The variant maps to a Fluent `error-open-*` message so the UI can render
/// a localized `$reason` for `error-open` instead of embedding hardcoded
/// English. `Unreadable` keeps the OS error text (not translated) as the
/// `{ $detail }` placeable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InputError {
    /// `std::fs::metadata` failed; holds the OS error text.
    Unreadable(String),
    /// The path is a directory, not a file.
    IsDir,
    /// The path exists but is not a regular file.
    NotFile,
    /// Larger than [`MAX_CAPTURE_BYTES`].
    TooLarge,
}

/// Validate a capture path before queuing it for indexing.
///
/// Best-effort UX pre-check only; real enforcement is the worker
/// streaming cap [`MAX_CAPTURE_BYTES`].
///
/// Returns `Ok` when the file can be opened, `Err(InputError)` otherwise.
/// A non-`.rif` extension is only a warning at the dialog level, so it
/// still validates here.
pub(crate) fn validate_input_path(path: &Path) -> Result<(), InputError> {
    let metadata =
        std::fs::metadata(path).map_err(|error| InputError::Unreadable(error.to_string()))?;
    if metadata.is_dir() {
        return Err(InputError::IsDir);
    }
    if !metadata.is_file() {
        return Err(InputError::NotFile);
    }
    if metadata.len() > MAX_CAPTURE_BYTES {
        return Err(InputError::TooLarge);
    }
    Ok(())
}

/// Validate a persisted update-check ETag.
///
/// Accepts the value when it is non-empty, at most 2048 bytes and only
/// printable ASCII (`0x20..=0x7E`, which already excludes `\r` and `\n`);
/// anything else is discarded as `None` so a corrupt storage entry never
/// reaches the network layer. Surrounding whitespace is trimmed to match
/// `crate::update::sanitize_etag`; the 2048-byte cap and the charset are
/// unchanged, and the network-side parser is untouched.
pub(crate) fn sanitize_etag_value(value: &str) -> Option<String> {
    if value.len() > 2048 {
        return None;
    }
    if !value.bytes().all(|byte| (0x20..=0x7E).contains(&byte)) {
        return None;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Application state.
pub struct Viewer {
    worker: Worker,
    limits: CaptureLimits,
    i18n: I18n,
    stage: Stage,
    last_time: f64,
    fade_at: Option<f64>,

    // Opening animation.
    scan_started: f64,
    scan_min_until: Option<f64>,
    read_received: u64,
    read_total: Option<u64>,
    home_error: Option<String>,

    // Capture.
    capture: Option<Arc<Capture>>,
    source: Option<PathBuf>,
    /// Owned labels of [`Self::capture`], cached at index time so the module
    /// filter can match without rebuilding a `Vec<String>` on every keystroke.
    labels: Vec<String>,
    filter: String,
    visible: Vec<usize>,
    selected: Option<usize>,
    body: Option<Arc<BodyView>>,
    /// Expanded text and line index per module index, so revisiting a module
    /// is an `Arc` clone instead of a re-expansion and a re-index.
    views: HashMap<usize, Arc<BodyView>>,
    /// Insertion order of [`Self::views`] keys, for bounded eviction.
    view_order: VecDeque<usize>,
    /// Retained text bytes currently held by [`Self::views`].
    view_bytes: usize,
    find: String,
    matches: Vec<usize>,
    match_cursor: usize,
    scroll_to_line: Option<usize>,
    pending_expand: Option<usize>,
    /// Monotonic sequence for expansion requests; lets the worker drop stale
    /// arrivals when the user moves quickly between modules.
    expand_seq: u64,
    busy: Option<String>,
    notice: Option<Notice>,
    gutter: bool,
    rail_open: bool,
    find_state: FindState,
    /// Structural-notes banner state for the open capture.
    notes: NotesState,
    /// Query text for the search across all modules.
    search_query: String,
    /// Search across all modules; `Some` while its panel is open.
    search: Option<GlobalSearch>,
    /// Module the user selected while a global search owned the worker.
    deferred_view: Option<usize>,
    /// A captured panic waiting for the user to dismiss it, if any.
    panic_report: Option<crate::panic::PanicReport>,

    // Preferences (persisted; see `settings::restore_prefs`).
    /// Light/dark/system theme preference.
    theme_pref: ThemePreference,
    /// Selected language tag; empty means "follow the system locale".
    language: String,
    /// The settings modal: `None` when hidden, otherwise the selected tab.
    settings: Option<SettingsTab>,
    /// Lazily uploaded texture for the GitHub mark shown in About.
    github_texture: Option<egui::TextureHandle>,

    // In-app updater (the only network path; see `crate::update`).
    /// Daily automatic checks enabled (persisted).
    update_auto: bool,
    /// Last completed check in Unix seconds (persisted).
    update_last_check: Option<u64>,
    /// Release tag the user chose to skip (persisted).
    update_skipped: String,
    /// Latest release newer than the running app, if one was found.
    update_release: Option<ReleaseInfo>,
    /// Verified installer waiting for the handoff, with the SHA-256 digest it
    /// was verified against (re-checked immediately before use).
    update_ready: Option<(PathBuf, String)>,
    /// Last update failure to show (manual checks and downloads only).
    update_error: Option<String>,
    /// Whether the last completed check found the app up to date (`None` while
    /// a check runs, after a failure, or before the first one ever runs).
    update_up_to_date: Option<bool>,
    /// ETag of the last release-metadata response (persisted), so a repeated
    /// check can answer `304` instead of consuming the API rate limit.
    update_etag: Option<String>,
    /// Background check state machine (at most one check runs at a time).
    check: CheckState,
    /// Background download state machine.
    download: DownloadState,
    /// Download progress: bytes written and advertised total, if known.
    update_download: Option<(u64, Option<u64>)>,
}

impl Viewer {
    /// Build the viewer and open a capture passed on the command line, if any.
    #[must_use]
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);

        let mut viewer = Self {
            worker: Worker::spawn(),
            limits: CaptureLimits::default(),
            i18n: I18n::detected(),
            stage: Stage::Welcome,
            last_time: 0.0,
            fade_at: None,
            scan_started: 0.0,
            scan_min_until: None,
            read_received: 0,
            read_total: None,
            home_error: None,
            capture: None,
            source: None,
            labels: Vec::new(),
            filter: String::new(),
            visible: Vec::new(),
            selected: None,
            body: None,
            views: HashMap::new(),
            view_order: VecDeque::new(),
            view_bytes: 0,
            find: String::new(),
            matches: Vec::new(),
            match_cursor: 0,
            scroll_to_line: None,
            pending_expand: None,
            expand_seq: 0,
            busy: None,
            notice: None,
            gutter: true,
            rail_open: true,
            find_state: FindState::Closed,
            notes: NotesState::Collapsed,
            search_query: String::new(),
            search: None,
            deferred_view: None,
            panic_report: None,
            theme_pref: ThemePreference::System,
            language: String::new(),
            settings: None,
            github_texture: None,
            update_auto: true,
            update_last_check: None,
            update_skipped: String::new(),
            update_release: None,
            update_ready: None,
            update_error: None,
            update_up_to_date: None,
            update_etag: None,
            check: CheckState::Idle,
            download: DownloadState::Idle,
            update_download: None,
        };

        viewer.restore_prefs(cc.storage, &cc.egui_ctx);
        // Discard a corrupt persisted ETag instead of sending it to the network.
        viewer.update_etag = viewer
            .update_etag
            .take()
            .and_then(|value| sanitize_etag_value(&value));

        if let Some(argument) = std::env::args_os().nth(1) {
            viewer.start_scan(PathBuf::from(argument));
        }
        if update::should_auto_check(viewer.update_auto) {
            viewer.start_check(false);
        }
        viewer
    }

    fn set_notice(&mut self, message: String, is_error: bool) {
        self.notice = Some(Notice { message, is_error });
    }

    /// Format a message that takes a `$path` and a `$reason`.
    fn message_with_path(&self, id: &str, path: &Path, reason: &str) -> String {
        let mut args = FluentArgs::new();
        args.set("path", path.display().to_string());
        args.set("reason", reason);
        self.i18n.render(id, &args)
    }

    /// Localized `$reason` for `error-open` from an [`InputError`].
    fn input_reason(&self, error: &InputError) -> String {
        match error {
            InputError::Unreadable(detail) => {
                let mut args = FluentArgs::new();
                args.set("detail", detail.clone());
                self.i18n.render("error-open-unreadable", &args)
            }
            InputError::IsDir => self.i18n.text("error-open-dir"),
            InputError::NotFile => self.i18n.text("error-open-not-file"),
            InputError::TooLarge => self.i18n.text("error-open-large"),
        }
    }

    /// Full `error-open` message for a path rejected by validation.
    fn open_error(&self, path: &Path, error: &InputError) -> String {
        let reason = self.input_reason(error);
        self.message_with_path("error-open", path, &reason)
    }

    /// Enter the opening animation and queue the capture for reading.
    fn start_scan(&mut self, path: PathBuf) {
        if let Err(error) = validate_input_path(&path) {
            self.stage = Stage::Home;
            self.fade_at = Some(self.last_time);
            let message = self.open_error(&path, &error);
            self.home_error = Some(message);
            return;
        }
        self.stage = Stage::Scanning;
        self.fade_at = Some(self.last_time);
        self.home_error = None;
        self.capture = None;
        self.labels.clear();
        self.body = None;
        self.views.clear();
        self.view_order.clear();
        self.view_bytes = 0;
        self.matches.clear();
        self.visible.clear();
        self.selected = None;
        self.pending_expand = None;
        self.busy = None;
        self.notice = None;
        self.notes = NotesState::Collapsed;
        self.search_query.clear();
        self.search = None;
        self.deferred_view = None;
        self.read_received = 0;
        self.read_total = None;
        self.scan_started = self.last_time;
        self.scan_min_until = Some(self.last_time + SCAN_MIN_SECONDS);
        self.source = Some(path.clone());
        self.worker.cancel();
        self.worker.index_with_limits(path, self.limits);
    }

    /// Ask the worker to expand a module.
    fn request_expand(&mut self, index: usize) {
        let Some(capture) = self.capture.clone() else {
            return;
        };
        self.pending_expand = Some(index);
        self.busy = Some(self.i18n.text("status-decoding"));
        self.expand_seq = self.expand_seq.wrapping_add(1);
        let seq = self.expand_seq;
        self.worker
            .expand_with_seq(capture, index, self.limits, seq);
    }

    /// Select a module and show it, expanding only on a cache miss.
    fn select(&mut self, index: usize) {
        self.selected = Some(index);
        if let Some(view) = self.cached_view(index) {
            self.pending_expand = None;
            if !self.search.as_ref().is_some_and(GlobalSearch::is_scanning) {
                self.busy = None;
            }
            self.show_view(view);
            return;
        }
        if self.search.as_ref().is_some_and(GlobalSearch::is_scanning) {
            // The search owns the worker; remember the choice and honour it
            // once the scan settles.
            self.pending_expand = None;
            self.deferred_view = Some(index);
            self.body = None;
            return;
        }
        self.request_expand(index);
    }

    /// Show a cached view without touching the worker.
    fn show_view(&mut self, view: Arc<BodyView>) {
        self.body = Some(view);
        self.scroll_to_line = Some(0);
        self.recompute_matches();
    }

    /// Cached expanded view for `index`, if the cache still holds it.
    fn cached_view(&self, index: usize) -> Option<Arc<BodyView>> {
        self.views.get(&index).map(Arc::clone)
    }

    /// Apply every event that arrived since the last frame.
    fn drain(&mut self) {
        loop {
            let Some(event) = self.worker.poll() else {
                return;
            };
            // `Event` is owned by the worker and is only ever extended. Every
            // current variant is handled; the wildcard keeps the thread
            // compiling when an additive variant lands before it is handled.
            #[allow(unreachable_patterns)]
            match event {
                Event::Reading {
                    received, total, ..
                } => {
                    self.read_received = received;
                    self.read_total = total;
                }
                Event::Indexed { path, capture, .. } => {
                    let modules = units(capture.len());
                    self.labels = capture
                        .parts()
                        .iter()
                        .map(|part| part.label().to_owned())
                        .collect();
                    self.capture = Some(capture);
                    self.source = Some(path);
                    self.scan_min_until = Some(
                        (self.scan_started + SCAN_MIN_SECONDS)
                            .max(self.last_time + SCAN_HOLD_SECONDS),
                    );
                    // A new index invalidates every cached view.
                    self.views.clear();
                    self.view_order.clear();
                    self.view_bytes = 0;
                    self.notes = NotesState::Collapsed;
                    let message = self.i18n.count("status-indexed", modules);
                    self.set_notice(message, false);
                }
                Event::IndexFailed { path, reason, .. } => {
                    self.stage = Stage::Home;
                    self.fade_at = Some(self.last_time);
                    self.capture = None;
                    self.labels.clear();
                    self.set_notice(String::new(), false);
                    let message = self.message_with_path("error-open", &path, &reason);
                    self.home_error = Some(message);
                }
                Event::Expanded { index, text, .. } => self.on_expanded(index, text),
                Event::ExpandFailed { index, reason, .. } => {
                    self.on_expand_failed(index, reason);
                }
                Event::ExpandProgress {
                    index,
                    received,
                    total,
                } => {
                    // The parser has no incremental expander yet, so this is a
                    // coarse signal with `received == 0` and `total == None`;
                    // once a size is known, the footer reports real progress.
                    if self.pending_expand == Some(index)
                        && let Some(total) = total.filter(|total| *total > 0)
                    {
                        let mut args = FluentArgs::new();
                        args.set("done", human_size_u64(received));
                        args.set("total", human_size_u64(total));
                        self.busy = Some(self.i18n.render("detail-read-of", &args));
                    }
                }
                Event::ExpandCancelled { index } => self.on_expand_cancelled(index),
                _ => {}
            }
        }
    }

    /// Apply one successful expansion to the view or to the running search.
    fn on_expanded<T: Into<Arc<PartText>>>(&mut self, index: usize, text: T) {
        let text = text.into();
        if self.search.as_ref().and_then(|search| search.in_flight) == Some(index) {
            let hit = self.search.as_ref().is_some_and(|search| {
                find_case_insensitive(&text.text, &search.needle, 0).is_some()
            });
            if let Some(search) = self.search.as_mut() {
                search.in_flight = None;
                search.scanned += 1;
                if hit {
                    search.hits.insert(index);
                }
            }
            self.advance_search();
            return;
        }
        if self.pending_expand == Some(index) {
            self.pending_expand = None;
            self.busy = None;
            self.set_body(index, text);
        }
    }

    /// Apply one failed expansion to the view or to the running search.
    fn on_expand_failed(&mut self, index: usize, reason: String) {
        if self.search.as_ref().and_then(|search| search.in_flight) == Some(index) {
            if let Some(search) = self.search.as_mut() {
                search.in_flight = None;
                search.scanned += 1;
            }
            self.advance_search();
            return;
        }
        if self.pending_expand == Some(index) {
            self.pending_expand = None;
            self.busy = None;
            self.body = None;
            let mut args = FluentArgs::new();
            args.set("index", to_number(units(index)));
            args.set("reason", reason);
            let message = self.i18n.render("error-module", &args);
            self.set_notice(message, true);
        }
    }

    /// Settle the view after an expansion was cancelled.
    fn on_expand_cancelled(&mut self, index: usize) {
        if self.search.as_ref().and_then(|search| search.in_flight) == Some(index) {
            if let Some(search) = self.search.as_mut() {
                search.in_flight = None;
                search.done = true;
            }
            self.settle_search();
            return;
        }
        if self.pending_expand == Some(index) {
            self.pending_expand = None;
            self.busy = None;
        }
    }

    /// Store an expanded module so a later revisit is an `Arc` clone.
    fn set_body<T: Into<Arc<PartText>>>(&mut self, index: usize, text: T) {
        let text = text.into();
        // One kernel yields both the line starts and the longest line in
        // characters, exactly the pair the reading surface scrolls against.
        let (line_starts, max_line_chars) = compute_view(&text.text);
        let view = Arc::new(BodyView {
            text,
            line_starts,
            max_line_chars,
        });
        self.remember_view(index, &view);
        self.show_view(view);
    }

    /// Insert a view into the bounded cache, evicting the oldest entries.
    fn remember_view(&mut self, index: usize, view: &Arc<BodyView>) {
        let size = view.text.text.len();
        if let Some(old) = self.views.insert(index, Arc::clone(view)) {
            self.view_bytes = self.view_bytes.saturating_sub(old.text.text.len());
        } else {
            self.view_order.push_back(index);
        }
        self.view_bytes = self.view_bytes.saturating_add(size);
        while self.view_order.len() > VIEW_CACHE_ENTRIES || self.view_bytes > VIEW_CACHE_BYTES {
            let Some(oldest) = self.view_order.pop_front() else {
                break;
            };
            if let Some(removed) = self.views.remove(&oldest) {
                self.view_bytes = self.view_bytes.saturating_sub(removed.text.text.len());
            }
        }
    }

    /// Start a search across every readable module of the open capture.
    fn start_global_search(&mut self) {
        let Some(capture) = self.capture.clone() else {
            return;
        };
        let needle = self.search_query.trim().to_owned();
        if needle.is_empty() {
            self.search = Some(GlobalSearch::idle());
            return;
        }
        let pending: VecDeque<usize> = capture
            .parts()
            .iter()
            .enumerate()
            .filter(|(_, part)| part.is_readable())
            .map(|(index, _)| index)
            .collect();
        let total = pending.len();
        self.search = Some(GlobalSearch {
            needle,
            pending,
            in_flight: None,
            hits: HashSet::new(),
            scanned: 0,
            total,
            done: total == 0,
        });
        self.deferred_view = None;
        self.advance_search();
    }

    /// Queue the next search expansion, or settle once the list drains.
    fn advance_search(&mut self) {
        if self.search.as_ref().is_none_or(|search| search.done) {
            self.settle_search();
            return;
        }
        let Some(capture) = self.capture.clone() else {
            if let Some(search) = self.search.as_mut() {
                search.done = true;
            }
            self.settle_search();
            return;
        };
        let next = {
            let Some(search) = self.search.as_mut() else {
                return;
            };
            if search.in_flight.is_some() {
                return;
            }
            if let Some(index) = search.pending.pop_front() {
                search.in_flight = Some(index);
                Some(index)
            } else {
                search.done = true;
                None
            }
        };
        let Some(index) = next else {
            self.settle_search();
            return;
        };
        self.expand_seq = self.expand_seq.wrapping_add(1);
        let seq = self.expand_seq;
        self.busy = Some(self.search_progress());
        self.worker
            .expand_with_seq(capture, index, self.limits, seq);
    }

    /// Progress text for the running search.
    fn search_progress(&self) -> String {
        match &self.search {
            Some(search) => self.i18n.progress(
                "search-all-progress",
                units(search.scanned),
                units(search.total),
            ),
            None => String::new(),
        }
    }

    /// Mark the search finished, report the hit count and resume any deferred
    /// module selection.
    fn settle_search(&mut self) {
        self.busy = None;
        let Some(search) = &self.search else {
            return;
        };
        let message = if search.hits.is_empty() {
            self.i18n.text("search-all-no-hits")
        } else {
            self.i18n.count("search-all-hits", units(search.hits.len()))
        };
        self.set_notice(message, false);
        if let Some(index) = self.deferred_view.take()
            && self.selected == Some(index)
        {
            self.request_expand(index);
        }
    }

    /// Cancel the running search and close its panel.
    fn close_global_search(&mut self) {
        self.search = None;
        self.busy = None;
        // Stop an in-flight inflation instead of letting it run to completion.
        self.worker.cancel_expand();
        if let Some(index) = self.deferred_view.take()
            && self.selected == Some(index)
        {
            self.request_expand(index);
        }
    }

    fn refresh_visible(&mut self) {
        if self.capture.is_none() {
            self.visible.clear();
            return;
        }
        // `self.labels` mirrors the capture's parts one-to-one (built at index
        // time), so the tested filter kernel drives the rail without rebuilding
        // a `Vec<String>` of labels on every keystroke.
        self.visible = filter_parts(&self.labels, &self.filter);
    }

    /// Move from the animation to the workspace once indexing is done.
    fn advance_stage(&mut self) {
        if self.stage != Stage::Scanning {
            return;
        }
        let Some(until) = self.scan_min_until else {
            return;
        };
        if self.capture.is_none() || self.last_time < until {
            return;
        }

        self.stage = Stage::Workspace;
        self.fade_at = Some(self.last_time);
        self.refresh_visible();
        if let Some(first) = self
            .capture
            .as_ref()
            .and_then(|capture| capture.parts().iter().position(Part::is_readable))
        {
            self.select(first);
        }
    }

    fn open_dialog(&mut self) {
        let filter = self.i18n.text("dialog-filter-name");
        let title = self.i18n.text("dialog-open-title");
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(filter, &["rif"])
            .set_title(title)
            .pick_file()
        {
            if let Err(error) = validate_input_path(&path) {
                self.stage = Stage::Home;
                self.fade_at = Some(self.last_time);
                let message = self.open_error(&path, &error);
                self.home_error = Some(message);
                return;
            }
            self.start_scan(path);
        }
    }

    fn suggested_file_name(&self) -> String {
        let stem = self
            .source
            .as_deref()
            .and_then(Path::file_stem)
            .map_or_else(
                || "capture".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            );
        let name = self.capture.as_ref().zip(self.selected).map_or_else(
            || "module".to_owned(),
            |(capture, index)| {
                let truncated: String = capture.parts()[index].label().chars().take(64).collect();
                sanitize(&truncated)
            },
        );
        format!("{stem}__{name}.txt")
    }

    fn copy_body(&mut self, ctx: &egui::Context) {
        if let Some(body) = &self.body {
            let bytes = units(body.text.text.len());
            let text = body.text.text.clone();
            ctx.copy_text(text);
            let message = self.i18n.count("status-copied", bytes);
            self.set_notice(message, false);
        }
    }

    fn save_body(&mut self) {
        let default_name = self.suggested_file_name();
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        let outcome = match self.body.as_ref() {
            Some(body) => {
                std::fs::write(&path, body.text.text.as_bytes()).map_err(|error| error.to_string())
            }
            None => return,
        };
        match outcome {
            Ok(()) => {
                let mut args = FluentArgs::new();
                args.set("path", path.display().to_string());
                let message = self.i18n.render("status-saved", &args);
                self.set_notice(message, false);
            }
            Err(error) => {
                let mut args = FluentArgs::new();
                args.set("reason", error);
                let message = self.i18n.render("error-save", &args);
                self.set_notice(message, true);
            }
        }
    }

    fn handle_dropped(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .map(|file| file.path().to_path_buf())
                .collect::<Vec<_>>()
        });
        if let Some(path) = dropped.into_iter().next() {
            if let Err(error) = validate_input_path(&path) {
                self.stage = Stage::Home;
                self.fade_at = Some(self.last_time);
                let message = self.open_error(&path, &error);
                self.home_error = Some(message);
                return;
            }
            self.start_scan(path);
        }
    }

    /// Request keyboard focus for the module filter field.
    fn focus_filter(&mut self, ui: &mut egui::Ui) {
        if self.stage != Stage::Workspace || self.find_state.is_open() || !self.rail_open {
            return;
        }
        if ui.memory(|memory| memory.focused().is_some()) {
            return;
        }
        ui.memory_mut(|memory| {
            memory.request_focus(egui::Id::new("module-filter"));
        });
    }

    /// Step the in-module match cursor, mirroring the find-bar navigation.
    fn step_match(&mut self, step: i32) {
        if self.stage != Stage::Workspace || self.matches.is_empty() {
            return;
        }
        let total = self.matches.len();
        if let Some(cursor) = next_match(total, self.match_cursor, step) {
            self.match_cursor = cursor;
            self.scroll_to_line = self.matches.get(self.match_cursor).copied();
        }
    }

    fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let (
            open_requested,
            save_requested,
            find_requested,
            settings_requested,
            escape,
            slash,
            next_requested,
        ) = ui.input(|input| {
            let command = input.modifiers.command;
            (
                command && input.key_pressed(egui::Key::O),
                command && input.key_pressed(egui::Key::S),
                command && input.key_pressed(egui::Key::F),
                command && input.key_pressed(egui::Key::Comma),
                input.key_pressed(egui::Key::Escape),
                input.key_pressed(egui::Key::Slash) && !command,
                input.key_pressed(egui::Key::F3) || (command && input.key_pressed(egui::Key::G)),
            )
        });
        if settings_requested {
            self.settings = Some(SettingsTab::General);
        }
        if open_requested {
            self.open_dialog();
        }
        if save_requested && self.body.is_some() {
            self.save_body();
        }
        if find_requested && self.stage == Stage::Workspace && self.selected.is_some() {
            self.find_state = FindState::Focus;
        }
        if escape && self.find_state.is_open() {
            self.find_state = FindState::Closed;
        }
        if slash {
            self.focus_filter(ui);
        }
        if next_requested && self.find_state.is_open() {
            self.step_match(1);
        }
    }

    fn footer_bar(&mut self, ui: &mut egui::Ui) {
        let palette = theme::current(ui.ctx());
        let ready = self.i18n.text("status-ready");
        let mut args = FluentArgs::new();
        args.set("version", env!("CARGO_PKG_VERSION"));
        args.set("os", theme::platform_name());
        args.set("arch", std::env::consts::ARCH);
        let footer = self.i18n.render("footer-build", &args);

        ui.horizontal(|ui| {
            if let Some(busy) = &self.busy {
                ui.spinner();
                ui.label(RichText::new(busy).size(12.0).color(palette.muted));
            } else if let Some(notice) = &self.notice {
                if notice.message.is_empty() {
                    ui.label(RichText::new(&ready).size(12.0).color(palette.muted));
                } else {
                    let color = if notice.is_error {
                        palette.danger
                    } else {
                        palette.muted
                    };
                    ui.label(RichText::new(&notice.message).size(12.0).color(color));
                }
            } else {
                ui.label(RichText::new(&ready).size(12.0).color(palette.muted));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(&footer).size(12.0).color(palette.muted));
            });
        });
    }

    /// Show the last-resort panic dialog while a captured report is pending.
    fn panic_dialog(&mut self, ctx: &egui::Context) {
        let mut dismissed = false;
        if let Some(report) = &self.panic_report {
            dismissed = crate::panic::show_dialog(ctx, report);
        }
        if dismissed {
            self.panic_report = None;
        }
    }

    fn draw_fade(&self, ctx: &egui::Context, palette: &Palette) {
        let Some(start) = self.fade_at else {
            return;
        };
        let elapsed = (self.last_time - start) as f32;
        if elapsed >= FADE_SECONDS {
            return;
        }
        let alpha = ((1.0 - elapsed / FADE_SECONDS) * 255.0).clamp(0.0, 255.0) as u8;
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("stage-fade"),
        ));
        painter.rect_filled(
            ctx.content_rect(),
            0.0,
            Color32::from_rgba_unmultiplied(
                palette.window.r(),
                palette.window.g(),
                palette.window.b(),
                alpha,
            ),
        );
    }
}

/// Frame for the splash stages (welcome/home/scanning), which paint a
/// full-bleed vertical gradient.
///
/// `Frame::central_panel` defaults to an 8 px inner margin filled with
/// `panel_fill` (pure white in light mode), so painting only
/// `available_rect_before_wrap()` left a visible ring ("banda") around the
/// bluish gradient. Zero margin + matching fill removes it.
fn splash_frame(style: &egui::Style, palette: &Palette) -> egui::Frame {
    egui::Frame::central_panel(style)
        .inner_margin(egui::Margin::ZERO)
        .fill(palette.bg_bottom)
}

impl Viewer {
    /// Settings entry, settings modal, stage fade and panic dialog, in order.
    fn overlays(&mut self, ctx: &egui::Context, palette: &Palette) {
        // The workspace header draws its own settings entry point; on the other
        // screens (welcome, home, the opening animation) a floating gear keeps
        // theme and language reachable before a capture is open.
        if self.stage != Stage::Workspace {
            self.settings_access(ctx);
        }
        self.settings_modal(ctx);
        self.changelog_window(ctx);
        self.draw_fade(ctx, palette);
        self.panic_dialog(ctx);
    }

    /// Render the current stage: splash screens or the module workspace.
    fn show_stage(&mut self, ui: &mut egui::Ui, palette: &Palette, file_hovered: bool) {
        let splash = splash_frame(ui.style(), palette);
        match self.stage {
            Stage::Welcome => {
                let mut started = false;
                egui::CentralPanel::default().frame(splash).show(ui, |ui| {
                    theme::paint_background(ui.painter(), ui.available_rect_before_wrap(), palette);
                    started = splash::welcome(ui, &self.i18n, palette);
                });
                if started {
                    self.stage = Stage::Home;
                    self.fade_at = Some(self.last_time);
                }
            }
            Stage::Home => {
                let error = self.home_error.clone();
                let mut chosen = false;
                egui::CentralPanel::default().frame(splash).show(ui, |ui| {
                    theme::paint_background(ui.painter(), ui.available_rect_before_wrap(), palette);
                    chosen = splash::home(ui, &self.i18n, palette, file_hovered);
                    if let Some(message) = &error {
                        ui.add_space(10.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(message).size(13.0).color(palette.danger));
                        });
                    }
                });
                if chosen {
                    self.open_dialog();
                }
            }
            Stage::Scanning => {
                let (phase, detail, progress) = self.scan_labels();
                egui::CentralPanel::default().frame(splash).show(ui, |ui| {
                    theme::paint_background(ui.painter(), ui.available_rect_before_wrap(), palette);
                    let view = ScanView {
                        elapsed: (self.last_time - self.scan_started) as f32,
                        progress,
                        phase: &phase,
                        detail: &detail,
                    };
                    splash::scanning(ui, &view, palette);
                });
            }
            Stage::Workspace => {
                egui::Panel::top("header").show(ui, |ui| self.header(ui));
                if self.rail_open {
                    egui::Panel::left("module_rail")
                        .resizable(true)
                        .default_size(280.0)
                        .size_range(220.0..=620.0)
                        .show(ui, |ui| self.module_rail(ui));
                }
                let content_frame = egui::Frame::central_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(14, 6));
                egui::CentralPanel::default()
                    .frame(content_frame)
                    .show(ui, |ui| self.reading_surface(ui));
            }
        }
    }
}

impl eframe::App for Viewer {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        let auto = if self.update_auto { "1" } else { "0" };
        storage.set_string(UPDATE_AUTO_KEY, auto.to_owned());
        if let Some(last) = self.update_last_check {
            storage.set_string(UPDATE_LAST_KEY, last.to_string());
        }
        storage.set_string(UPDATE_SKIPPED_KEY, self.update_skipped.clone());
        storage.set_string(
            UPDATE_ETAG_KEY,
            self.update_etag.clone().unwrap_or_default(),
        );
        storage.set_string(
            THEME_KEY,
            settings::theme_storage_key(self.theme_pref).to_owned(),
        );
        storage.set_string(LANGUAGE_KEY, self.language.clone());
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.last_time = ui.input(|input| input.time);
        self.drain();
        self.poll_update();
        self.handle_dropped(ui.ctx());
        // The settings modal consumes its own keys (Esc closes it); letting the
        // workspace shortcuts run underneath would close two things at once.
        if self.settings.is_none() {
            self.handle_shortcuts(ui);
        }
        self.advance_stage();
        if let Some(report) = crate::panic::take_report() {
            self.panic_report = Some(report);
        }

        let palette = theme::current(ui.ctx());
        let file_hovered = ui.input(|input| !input.raw.hovered_files.is_empty());
        let footer_frame =
            egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(10, 3));
        egui::Panel::bottom("footer")
            .frame(footer_frame)
            .show(ui, |ui| self.footer_bar(ui));

        self.show_stage(ui, &palette, file_hovered);

        self.overlays(ui.ctx(), &palette);
        let fade_active = self
            .fade_at
            .is_some_and(|start| self.last_time - start < f64::from(FADE_SECONDS));
        if fade_active
            || self.busy.is_some()
            || self.stage == Stage::Scanning
            || self.search.as_ref().is_some_and(GlobalSearch::is_scanning)
            || !matches!(self.check, CheckState::Idle)
            || !matches!(self.download, DownloadState::Idle)
        {
            ui.ctx().request_repaint();
        }
    }
}

impl Viewer {
    /// Phase text, detail text and progress fraction for the scanning view.
    fn scan_labels(&self) -> (String, String, Option<f32>) {
        if let Some(capture) = &self.capture {
            return (
                self.i18n.text("phase-preparing"),
                self.i18n
                    .count("detail-modules-found", units(capture.len())),
                Some(1.0),
            );
        }

        let progress = self
            .read_total
            .filter(|total| *total > 0)
            .map(|total| (self.read_received as f32 / total as f32).clamp(0.0, 1.0));

        match (self.read_total, progress) {
            (Some(total), Some(fraction)) if self.read_received >= total => (
                self.i18n.text("phase-indexing"),
                self.i18n.text("detail-indexing"),
                Some(fraction),
            ),
            (Some(total), Some(fraction)) => {
                let mut args = FluentArgs::new();
                args.set("done", human_size_u64(self.read_received));
                args.set("total", human_size_u64(total));
                (
                    self.i18n.text("phase-reading"),
                    self.i18n.render("detail-read-of", &args),
                    Some(fraction),
                )
            }
            _ => {
                let mut args = FluentArgs::new();
                args.set("size", human_size_u64(self.read_received));
                (
                    self.i18n.text("phase-reading"),
                    self.i18n.render("detail-read", &args),
                    None,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "mikrotik-rif-app-test-{}-{seq}-{tag}.rif",
            std::process::id()
        ))
    }

    #[test]
    fn valid_file_passes_validation() {
        let path = temp_path("ok");
        std::fs::write(&path, b"data").unwrap();
        assert!(validate_input_path(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_file_is_rejected() {
        let path = std::env::temp_dir().join(format!(
            "mikrotik-rif-app-test-missing-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        assert!(validate_input_path(&path).is_err());
    }

    #[test]
    fn directory_is_rejected() {
        let dir = std::env::temp_dir();
        assert!(validate_input_path(&dir).is_err());
    }

    #[test]
    fn non_rif_extension_is_still_allowed() {
        let mut path = temp_path("txt");
        path.set_extension("txt");
        std::fs::write(&path, b"data").unwrap();
        assert!(validate_input_path(&path).is_ok());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn oversized_file_is_rejected_with_the_worker_reason() {
        let path = temp_path("big");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_CAPTURE_BYTES + 1).unwrap();
        drop(file);
        assert_eq!(validate_input_path(&path), Err(InputError::TooLarge));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn etag_allows_printable_ascii_up_to_the_cap() {
        assert_eq!(
            sanitize_etag_value("\"abc123\""),
            Some("\"abc123\"".to_owned())
        );
        assert_eq!(
            sanitize_etag_value("W/\"xyz\""),
            Some("W/\"xyz\"".to_owned())
        );
    }

    #[test]
    fn etag_rejects_empty_oversized_and_non_ascii() {
        assert_eq!(sanitize_etag_value(""), None);
        assert_eq!(sanitize_etag_value(&"a".repeat(2049)), None);
        assert_eq!(sanitize_etag_value("etag\r\ninjected"), None);
        assert_eq!(sanitize_etag_value("café"), None);
        assert_eq!(sanitize_etag_value("with space\x7F"), None);
    }

    #[test]
    fn idle_global_search_is_not_scanning() {
        let search = GlobalSearch::idle();
        assert!(!search.is_scanning());
        assert!(search.needle.is_empty());
        assert!(search.hits.is_empty());
        assert!(search.pending.is_empty());
        assert!(search.in_flight.is_none());
    }

    #[test]
    fn global_search_hit_test_matches_the_ascii_insensitive_matcher() {
        let needle = "eth";
        assert!(find_case_insensitive("Ether1", needle, 0).is_some());
        assert!(find_case_insensitive("no match here", needle, 0).is_none());
    }

    #[test]
    fn new_messages_parse_and_pluralize() {
        let i18n = I18n::for_language("en");
        assert_eq!(i18n.count("notes-warning", 1), "1 structural oddity found");
        assert_eq!(
            i18n.count("notes-warning", 3),
            "3 structural oddities found"
        );
        assert_eq!(i18n.count("search-all-hits", 1), "1 module matches");
        assert_eq!(i18n.count("search-all-hits", 4), "4 modules match");
        // A raw identifier would mean Fluent rejected the message and fell
        // through to the id fallback.
        assert_ne!(i18n.text("notes-details-show"), "notes-details-show");
        assert_ne!(i18n.text("search-all-no-hits"), "search-all-no-hits");
    }
}
