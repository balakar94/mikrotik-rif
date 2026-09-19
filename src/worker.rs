//! Background jobs so reading, indexing and expansion never block the window.
//!
//! A capture can be hundreds of megabytes, so the file read, the index pass and
//! the zlib expansion all run on a dedicated thread. The interface thread only
//! sends jobs and drains events once per frame. The read phase reports real
//! byte progress so the opening animation is driven by actual work.
//!
//! Cancellation is cooperative. [`Worker::cancel`] bumps a generation counter
//! *and* trips the cancellation token of the in-flight index or expansion, so a
//! long pass stops at its next checkpoint instead of running to completion.
//! Cancelled work is reported quietly: an aborted index emits neither
//! [`Event::Indexed`] nor [`Event::IndexFailed`], and an aborted expansion emits
//! [`Event::ExpandCancelled`] instead of [`Event::Expanded`].
//!
//! Expanded text is cached per capture in a [`PartCache`] owned by the worker
//! thread, so re-selecting a module is a cache hit rather than a second
//! inflation. The cache is keyed by part index alone, so it is cleared whenever
//! a new capture starts indexing.
//!
//! Wave2 notes: [`Event`] is only ever extended, never reshaped — `app.rs`
//! matches it exhaustively, so existing variants keep their names and fields and
//! new information arrives through new methods and variants only.

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;

use crate::parser::error::RifError;
use crate::parser::{Cancel, Capture, CaptureLimits, Part, PartCache, PartText};

/// Chunk size used while streaming the file from disk.
const READ_CHUNK: usize = 256 * 1024;

/// Progress is reported at most once per this many bytes.
const PROGRESS_STEP: u64 = 1024 * 1024;

/// Cap on the upfront buffer reservation, so a huge file cannot over-allocate.
const RESERVE_CAP: u64 = 64 * 1024 * 1024;

/// Hard cap on the capture bytes buffered in memory.
///
/// Checked twice: up front against the file metadata so an oversized file is
/// rejected without allocating, and again while streaming so a file that grows
/// mid-read (or reports a bogus size) still aborts instead of exhausting RAM.
pub const MAX_CAPTURE_BYTES: u64 = 512 * 1024 * 1024;

/// How often, in read chunks, the index pass checks for cancellation.
const CANCEL_CHECK_EVERY: u64 = 16;

/// Rejection reason shared by both oversized-file checks.
const TOO_LARGE_REASON: &str = "file too large (>512 MiB)";

/// Something the worker reported to the interface.
pub enum Event {
    /// Bytes of the capture read so far, and the total size when known.
    Reading {
        /// Bytes read so far.
        received: u64,
        /// Total file size, when the platform reported one.
        total: Option<u64>,
    },
    /// A capture was read and indexed.
    Indexed {
        /// File the capture came from.
        path: PathBuf,
        /// Indexed capture, shared with the interface thread.
        capture: Arc<Capture>,
    },
    /// A capture could not be read or indexed.
    IndexFailed {
        /// File that failed.
        path: PathBuf,
        /// Human-readable reason.
        reason: String,
    },
    /// One part was expanded into text.
    Expanded {
        /// Index of the expanded part.
        index: usize,
        /// Expanded text.
        text: PartText,
    },
    /// One part could not be expanded.
    ExpandFailed {
        /// Index of the part that failed.
        index: usize,
        /// Human-readable reason.
        reason: String,
    },
    /// A cache miss is about to inflate a part.
    ///
    /// The parser exposes only a blocking expansion primitive, so there is no
    /// incremental byte stream to report. This is deliberately a coarse signal
    /// — one event at the start of a miss, with `received` at zero and `total`
    /// unknown — and never a fabricated percentage. The decompressed size only
    /// becomes known when [`Event::Expanded`] carries the text.
    ExpandProgress {
        /// Index of the part being expanded.
        index: usize,
        /// Decompressed bytes observed so far; always zero with the current
        /// blocking parser API.
        received: u64,
        /// Decompressed size, when known; always `None` until the expansion
        /// finishes.
        total: Option<u64>,
    },
    /// An in-flight expansion was aborted by a cancellation token.
    ///
    /// A request superseded by a newer sequence is still dropped silently (no
    /// event); this variant is emitted only when the worker was asked to cancel
    /// the running expansion.
    ExpandCancelled {
        /// Index of the part whose expansion was cancelled.
        index: usize,
    },
}

enum Job {
    Index {
        path: PathBuf,
        limits: CaptureLimits,
    },
    Expand {
        capture: Arc<Capture>,
        index: usize,
        limits: CaptureLimits,
        seq: u64,
    },
    /// Bumps the index generation once the queued index jobs ahead of it have
    /// drained. Sent by [`Worker::cancel`] in addition to tripping the in-flight
    /// token directly.
    Cancel,
}

/// Read-only snapshot of the worker's expansion cache counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Number of parts currently cached.
    pub entries: usize,
    /// Retained text bytes currently cached.
    pub bytes: usize,
    /// Cache hits observed by the worker.
    pub hits: u64,
    /// Cache misses observed by the worker.
    pub misses: u64,
}

/// Cache counters written by the worker thread and read by the interface.
#[derive(Debug, Default)]
struct CacheCounters {
    entries: AtomicUsize,
    bytes: AtomicUsize,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl CacheCounters {
    /// Take a consistent-enough snapshot of every counter.
    fn snapshot(&self) -> CacheStats {
        CacheStats {
            entries: self.entries.load(Ordering::Relaxed),
            bytes: self.bytes.load(Ordering::Relaxed),
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
        }
    }
}

/// Worker-thread-only handles bundled together to keep the job helpers small.
struct WorkerChannels {
    events: Sender<Event>,
    latest_expand_seq: Arc<AtomicU64>,
    cache_counters: Arc<CacheCounters>,
}

/// Handle to the single worker thread that processes jobs in order.
pub struct Worker {
    jobs: Sender<Job>,
    events: Receiver<Event>,
    /// Generation bumped by every cancellation; the in-flight index pass
    /// snapshots it at start and abandons its work once the live value
    /// diverges. Per-worker so parallel test workers never share a generation.
    index_epoch: Arc<AtomicU64>,
    /// Highest expansion sequence seen so far. Queued expansions carrying an
    /// older non-zero sequence were superseded by a newer request and are
    /// skipped instead of decoded. Sequence `0` is the legacy path: it never
    /// updates this counter and is never dropped. Per-worker so parallel test
    /// workers never share a sequence.
    latest_expand_seq: Arc<AtomicU64>,
    /// Cancellation token of the in-flight index pass, if any. Installed by the
    /// worker thread at job start and cleared at job end; [`Worker::cancel`]
    /// trips it immediately.
    index_cancel: Arc<Mutex<Option<Cancel>>>,
    /// Cancellation token of the in-flight expansion, if any. Same lifecycle as
    /// [`Worker::index_cancel`].
    expand_cancel: Arc<Mutex<Option<Cancel>>>,
    /// Expansion cache counters, shared with the worker thread for a lock-free
    /// [`Worker::cache_stats`].
    cache_counters: Arc<CacheCounters>,
}

impl Worker {
    /// Start the worker thread.
    #[must_use]
    pub fn spawn() -> Self {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (events_tx, events_rx) = mpsc::channel::<Event>();
        let index_epoch = Arc::new(AtomicU64::new(0));
        let latest_expand_seq = Arc::new(AtomicU64::new(0));
        let index_cancel = Arc::new(Mutex::new(None));
        let expand_cancel = Arc::new(Mutex::new(None));
        let cache_counters = Arc::new(CacheCounters::default());

        let index_epoch_thread = Arc::clone(&index_epoch);
        let index_cancel_thread = Arc::clone(&index_cancel);
        let expand_cancel_thread = Arc::clone(&expand_cancel);
        let channels = WorkerChannels {
            events: events_tx,
            latest_expand_seq: Arc::clone(&latest_expand_seq),
            cache_counters: Arc::clone(&cache_counters),
        };

        thread::Builder::new()
            .name("mikrotik-rif-worker".to_owned())
            .spawn(move || {
                // The cache is keyed by part index, so it belongs to whichever
                // capture was indexed last; only this thread ever touches it.
                let mut cache = PartCache::default();
                while let Ok(job) = jobs_rx.recv() {
                    let keep_going = match job {
                        Job::Index { path, limits } => {
                            // A new capture invalidates every cached part: the
                            // indices now refer to different modules.
                            cache.clear();
                            channels.cache_counters.entries.store(0, Ordering::Relaxed);
                            channels.cache_counters.bytes.store(0, Ordering::Relaxed);

                            let cancel = Cancel::new();
                            *lock_cancel(&index_cancel_thread) = Some(cancel.clone());
                            let epoch = index_epoch_thread.load(Ordering::SeqCst);
                            let keep = index_with_progress(
                                &path,
                                &limits,
                                epoch,
                                &index_epoch_thread,
                                &cancel,
                                &channels,
                            );
                            *lock_cancel(&index_cancel_thread) = None;
                            keep
                        }
                        Job::Expand {
                            capture,
                            index,
                            limits,
                            seq,
                        } => {
                            let cancel = Cancel::new();
                            *lock_cancel(&expand_cancel_thread) = Some(cancel.clone());
                            let keep = expand_with_cache(
                                &capture, index, &limits, seq, &mut cache, &cancel, &channels,
                            );
                            *lock_cancel(&expand_cancel_thread) = None;
                            keep
                        }
                        Job::Cancel => {
                            index_epoch_thread.fetch_add(1, Ordering::SeqCst);
                            true
                        }
                    };
                    if !keep_going {
                        break;
                    }
                }
            })
            .expect("the worker thread must start");

        Self {
            jobs: jobs_tx,
            events: events_rx,
            index_epoch,
            latest_expand_seq,
            index_cancel,
            expand_cancel,
            cache_counters,
        }
    }

    /// Queue a capture to be read and indexed with default budgets.
    #[allow(
        dead_code,
        reason = "legacy compat wrapper; new code uses index_with_limits"
    )]
    pub fn index(&self, path: PathBuf) {
        self.index_with_limits(path, CaptureLimits::default());
    }

    /// Queue a capture to be read and indexed with explicit budgets.
    pub fn index_with_limits(&self, path: PathBuf, limits: CaptureLimits) {
        let _ = self.jobs.send(Job::Index { path, limits });
    }

    /// Queue a part to be expanded.
    ///
    /// Legacy path: sequence `0`, so the request is always decoded and the
    /// resulting event always delivered.
    #[allow(
        dead_code,
        reason = "legacy compat wrapper; new code uses expand_with_seq"
    )]
    pub fn expand(&self, capture: Arc<Capture>, index: usize, limits: CaptureLimits) {
        self.expand_with_seq(capture, index, limits, 0);
    }

    /// Queue a part to be expanded, superseding older queued expansions.
    ///
    /// `seq` should grow with every request (a per-view counter works); when a
    /// newer sequence is queued first, an older one still waiting is skipped
    /// instead of decoded. Pass `0` for the legacy always-decode behaviour.
    pub fn expand_with_seq(
        &self,
        capture: Arc<Capture>,
        index: usize,
        limits: CaptureLimits,
        seq: u64,
    ) {
        if seq != 0 {
            self.latest_expand_seq.fetch_max(seq, Ordering::SeqCst);
        }
        let _ = self.jobs.send(Job::Expand {
            capture,
            index,
            limits,
            seq,
        });
    }

    /// Ask the worker to abandon the in-flight index pass and expansion.
    ///
    /// Preemptive: bumps this worker's index generation immediately and trips
    /// the cancellation tokens of any in-flight index or expansion, so both
    /// observe the request at their next checkpoint and stop quietly. The
    /// queued [`Job::Cancel`] is only processed after the current job (the
    /// queue is FIFO behind a blocking `recv`); it bumps the generation a
    /// second time to drain any index already waiting behind the in-flight
    /// one, after which a later `index` starts a fresh generation.
    pub fn cancel(&self) {
        self.index_epoch.fetch_add(1, Ordering::SeqCst);
        trip_cancel(&self.index_cancel);
        trip_cancel(&self.expand_cancel);
        let _ = self.jobs.send(Job::Cancel);
    }

    /// Ask the worker to abandon only the in-flight expansion.
    ///
    /// Trips the expansion token so a large inflation aborts at its next read
    /// checkpoint. Queued expansions are governed by their sequence instead;
    /// a request already superseded is dropped anyway. Base plumbing only —
    /// the interface does not call this yet (Wave2 wiring).
    #[allow(
        dead_code,
        reason = "Wave2 wiring; callers cancel expansions via cancel() for now"
    )]
    pub fn cancel_expand(&self) {
        trip_cancel(&self.expand_cancel);
    }

    /// Snapshot the expansion cache counters.
    ///
    /// Read-only, lock-free and safe to call from the interface thread.
    #[allow(
        dead_code,
        reason = "Wave2 wiring; exposed for the interface status line"
    )]
    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        self.cache_counters.snapshot()
    }

    /// Take the next event, if one is waiting.
    #[must_use]
    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

/// Lock a cancellation slot, recovering from a poisoned mutex.
///
/// The worker never panics while holding the lock, so poisoning is not expected;
/// recovering keeps a hypothetical panic from silently disabling cancellation.
fn lock_cancel(slot: &Mutex<Option<Cancel>>) -> MutexGuard<'_, Option<Cancel>> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Trip the token in `slot`, if one is installed.
fn trip_cancel(slot: &Mutex<Option<Cancel>>) {
    let guard = lock_cancel(slot);
    if let Some(token) = guard.as_ref() {
        token.cancel();
    }
}

/// Whether `len` buffered bytes fit under the hard capture cap.
const fn size_allowed(len: u64) -> bool {
    len <= MAX_CAPTURE_BYTES
}

/// Stream the file from disk, reporting progress, then index it.
///
/// `epoch` is the worker's index generation the job started in and
/// `index_epoch` is the live per-[`Worker`] counter; the pass abandons its
/// work (quietly, keeping the worker alive) once the live generation diverges
/// or `cancel` is tripped, checked every [`CANCEL_CHECK_EVERY`] read chunks and
/// again per line/part by the parser. Progress events go out at most once per
/// [`PROGRESS_STEP`] bytes plus one final report, so `reported` only moves when
/// an event is really sent.
fn index_with_progress(
    path: &PathBuf,
    limits: &CaptureLimits,
    epoch: u64,
    index_epoch: &AtomicU64,
    cancel: &Cancel,
    channels: &WorkerChannels,
) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return channels
                .events
                .send(Event::IndexFailed {
                    path: path.clone(),
                    reason: format!("cannot read the file: {error}"),
                })
                .is_ok();
        }
    };

    let meta_len = file.metadata().ok().map(|meta| meta.len());
    if meta_len.is_some_and(|len| !size_allowed(len)) {
        return channels
            .events
            .send(Event::IndexFailed {
                path: path.clone(),
                reason: TOO_LARGE_REASON.to_owned(),
            })
            .is_ok();
    }
    let total = meta_len.filter(|size| *size > 0);
    let reserve = total.unwrap_or(0).min(RESERVE_CAP);
    let mut bytes: Vec<u8> = Vec::with_capacity(usize::try_from(reserve).unwrap_or(0));
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut received: u64 = 0;
    let mut reported: u64 = 0;
    let mut chunks: u64 = 0;

    loop {
        if chunks.is_multiple_of(CANCEL_CHECK_EVERY)
            && (index_epoch.load(Ordering::SeqCst) != epoch || cancel.is_cancelled())
        {
            return true;
        }
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                received += read as u64;
                if !size_allowed(received) {
                    return channels
                        .events
                        .send(Event::IndexFailed {
                            path: path.clone(),
                            reason: TOO_LARGE_REASON.to_owned(),
                        })
                        .is_ok();
                }
                if received - reported >= PROGRESS_STEP {
                    reported = received;
                    if channels
                        .events
                        .send(Event::Reading { received, total })
                        .is_err()
                    {
                        return false;
                    }
                }
                chunks += 1;
            }
            Err(error) => {
                return channels
                    .events
                    .send(Event::IndexFailed {
                        path: path.clone(),
                        reason: format!("cannot read the file: {error}"),
                    })
                    .is_ok();
            }
        }
    }

    if channels
        .events
        .send(Event::Reading { received, total })
        .is_err()
    {
        return false;
    }

    let event = match Capture::from_bytes_cancellable(&bytes, limits, cancel) {
        Ok(capture) => Event::Indexed {
            path: path.clone(),
            capture: Arc::new(capture),
        },
        // Cancellation is reported quietly, exactly like the epoch path: no
        // `Indexed` and no `IndexFailed`, and the worker stays alive.
        Err(RifError::Cancelled) => return true,
        Err(error) => Event::IndexFailed {
            path: path.clone(),
            reason: error.to_string(),
        },
    };
    channels.events.send(event).is_ok()
}

/// Expand one part, unless `seq` was superseded by a newer request.
///
/// Returns `true` while the event channel is healthy (the caller emits no event
/// for stale, cancelled or superseded work). Sequence `0` is the legacy path
/// and is always decoded. `channels.latest_expand_seq` is the owning
/// [`Worker`]'s highest sequence seen so far; `cache` is the worker-thread-only
/// expansion cache.
fn expand_with_cache(
    capture: &Capture,
    index: usize,
    limits: &CaptureLimits,
    seq: u64,
    cache: &mut PartCache,
    cancel: &Cancel,
    channels: &WorkerChannels,
) -> bool {
    if seq != 0 && seq < channels.latest_expand_seq.load(Ordering::SeqCst) {
        return true;
    }

    // `get` marks the entry most-recently-used; `read_cached` then re-validates
    // it against the current limits before returning it.
    let cached = cache.get(index).is_some();
    if cached {
        channels.cache_counters.hits.fetch_add(1, Ordering::Relaxed);
    } else {
        channels
            .cache_counters
            .misses
            .fetch_add(1, Ordering::Relaxed);
    }

    // Only a real miss on a readable part does work worth announcing. The
    // parser has no incremental expander, so this is a coarse "started" signal.
    let will_expand = !cached && capture.parts().get(index).is_some_and(Part::is_readable);
    if will_expand
        && channels
            .events
            .send(Event::ExpandProgress {
                index,
                received: 0,
                total: None,
            })
            .is_err()
    {
        return false;
    }

    match capture.read_cached(index, limits, cache, cancel) {
        Ok(text) => {
            channels
                .cache_counters
                .entries
                .store(cache.len(), Ordering::Relaxed);
            channels
                .cache_counters
                .bytes
                .store(cache.bytes(), Ordering::Relaxed);
            channels
                .events
                .send(Event::Expanded {
                    index,
                    text: Arc::unwrap_or_clone(text),
                })
                .is_ok()
        }
        Err(RifError::Cancelled) => channels
            .events
            .send(Event::ExpandCancelled { index })
            .is_ok(),
        Err(error) => channels
            .events
            .send(Event::ExpandFailed {
                index,
                reason: error.to_string(),
            })
            .is_ok(),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    use super::*;
    use crate::parser::codec;
    use crate::parser::scanner::{CLOSE_MARKER, OPEN_MARKER};

    /// Unique scratch file per test, since tests run in parallel.
    fn temp_path(tag: &str) -> PathBuf {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let seq = SEQUENCE.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!(
            "mikrotik-rif-worker-test-{}-{seq}-{tag}.rif",
            std::process::id()
        ))
    }

    /// Deterministic incompressible bytes: a 64-bit LCG spilling little-endian
    /// words, so zlib cannot find any repetition to exploit. Indexing and
    /// expansion then do real work instead of racing through a run of zeros.
    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        let mut out = Vec::with_capacity(len);
        while out.len() < len {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            out.extend_from_slice(&state.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    /// Encode one part the way the router would: label, NUL, zlib payload.
    fn encode_part(label: &[u8], body: &[u8]) -> Vec<u8> {
        let mut plain = label.to_vec();
        plain.push(0);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(body).unwrap();
        plain.extend_from_slice(&encoder.finish().unwrap());
        codec::pack(&plain)
    }

    /// Wrap encoded parts in the container markers.
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

    /// Smallest well-formed capture: one part holding `hello`.
    fn tiny_capture_bytes() -> Vec<u8> {
        wrap(&[encode_part(b"export", b"hello\n")])
    }

    /// Two-part capture, used as an expansion sentinel with a distinct index.
    fn two_part_capture_bytes() -> Vec<u8> {
        wrap(&[
            encode_part(b"first", b"one\n"),
            encode_part(b"second", b"two\n"),
        ])
    }

    /// Drain events until `f` resolves, giving up after a generous deadline.
    fn settle<T>(worker: &Worker, mut f: impl FnMut(Event) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = worker.poll()
                && let Some(done) = f(event)
            {
                return done;
            }
            assert!(
                Instant::now() <= deadline,
                "worker did not settle within the deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn capture_cap_is_512_mib() {
        assert_eq!(MAX_CAPTURE_BYTES, 512 * 1024 * 1024);
    }

    #[test]
    fn size_gate_accepts_up_to_the_cap() {
        assert!(size_allowed(0));
        assert!(size_allowed(MAX_CAPTURE_BYTES));
        assert!(!size_allowed(MAX_CAPTURE_BYTES + 1));
        assert!(!size_allowed(u64::MAX));
    }

    #[test]
    fn cancel_then_index_still_delivers_the_capture() {
        let path = temp_path("cancel");
        std::fs::write(&path, tiny_capture_bytes()).unwrap();

        let worker = Worker::spawn();
        worker.cancel();
        worker.index_with_limits(path.clone(), CaptureLimits::default());

        let capture = settle(&worker, |event| match event {
            Event::Indexed { capture, .. } => Some(capture),
            Event::IndexFailed { reason, .. } => panic!("index must succeed: {reason}"),
            Event::Reading { .. }
            | Event::Expanded { .. }
            | Event::ExpandFailed { .. }
            | Event::ExpandProgress { .. }
            | Event::ExpandCancelled { .. } => None,
        });
        assert_eq!(capture.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn superseded_expansion_is_skipped() {
        let bytes = tiny_capture_bytes();
        let limits = CaptureLimits::default();
        let capture = Arc::new(Capture::from_bytes(&bytes, &limits).unwrap());

        let worker = Worker::spawn();
        worker.expand_with_seq(capture.clone(), 0, limits, 5);
        worker.expand_with_seq(capture.clone(), 0, limits, 3);

        let first = settle(&worker, |event| match event {
            Event::Expanded { index, text } => Some((index, text)),
            Event::ExpandFailed { reason, .. } => panic!("expansion must succeed: {reason}"),
            Event::Reading { .. }
            | Event::Indexed { .. }
            | Event::IndexFailed { .. }
            | Event::ExpandProgress { .. }
            | Event::ExpandCancelled { .. } => None,
        });
        assert_eq!(first.0, 0);
        assert_eq!(first.1.text, "hello\n");

        std::thread::sleep(Duration::from_millis(500));
        assert!(
            worker.poll().is_none(),
            "the superseded request must emit no event"
        );
    }

    #[test]
    fn cancelled_index_emits_no_indexed() {
        // A capture large enough that indexing is still running when the
        // cancellation arrives; incompressible bodies keep the parse honest.
        let bytes = wrap(
            &(0..80_u64)
                .map(|seed| encode_part(b"bulk", &pseudo_random(64 * 1024, seed)))
                .collect::<Vec<_>>(),
        );
        assert!(
            bytes.len() > 4 * 1024 * 1024,
            "the cancellation window must span at least one read checkpoint"
        );
        let slow = temp_path("cancel-index");
        std::fs::write(&slow, bytes).unwrap();

        let sentinel = temp_path("cancel-index-sentinel");
        std::fs::write(&sentinel, tiny_capture_bytes()).unwrap();

        let worker = Worker::spawn();
        let limits = CaptureLimits::default();
        worker.index_with_limits(slow.clone(), limits);

        // Wait until the pass is provably in flight (real byte progress went
        // out) before asking it to stop.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() <= deadline, "index never reported progress");
            match worker.poll() {
                Some(Event::Reading { .. }) => break,
                Some(Event::Indexed { path, .. }) => {
                    panic!("index finished before cancellation: {}", path.display());
                }
                Some(Event::IndexFailed { reason, .. }) => panic!("index failed: {reason}"),
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        worker.cancel();

        // The sentinel is queued after the cancelled job; the worker is
        // single-threaded, so any `Indexed` from the slow capture would arrive
        // before the sentinel and fail the assertion.
        worker.index_with_limits(sentinel.clone(), limits);
        let capture = settle(&worker, |event| match event {
            Event::Indexed { path, capture } => {
                assert_ne!(path, slow, "a cancelled index must emit no Indexed");
                Some(capture)
            }
            Event::IndexFailed { path, reason } => {
                assert_ne!(path, slow, "a cancelled index must fail quietly: {reason}");
                panic!("sentinel index must succeed: {reason}");
            }
            Event::Reading { .. }
            | Event::Expanded { .. }
            | Event::ExpandFailed { .. }
            | Event::ExpandProgress { .. }
            | Event::ExpandCancelled { .. } => None,
        });
        assert_eq!(capture.len(), 1, "the sentinel capture must be indexed");

        let _ = std::fs::remove_file(&slow);
        let _ = std::fs::remove_file(&sentinel);
    }

    #[test]
    fn cancelled_expansion_emits_no_expanded() {
        // Incompressible data inflates slowly enough to catch the cancellation
        // while the blocking parser call is still running.
        let big = wrap(&[encode_part(
            b"big",
            &pseudo_random(16 * 1024 * 1024, 0x00C0_FFEE),
        )]);
        let limits = CaptureLimits::default();
        let big = Arc::new(Capture::from_bytes(&big, &limits).unwrap());

        let sentinel_bytes = two_part_capture_bytes();
        let sentinel = Arc::new(Capture::from_bytes(&sentinel_bytes, &limits).unwrap());

        let worker = Worker::spawn();
        worker.expand_with_seq(big.clone(), 0, limits, 7);

        // Wait for the coarse start signal: the expansion is in flight.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() <= deadline, "expansion never started");
            match worker.poll() {
                Some(Event::ExpandProgress { index: 0, .. }) => break,
                Some(Event::Expanded { index: 0, .. }) => {
                    panic!("expansion finished before cancellation");
                }
                Some(Event::ExpandCancelled { index: 0 }) => {
                    panic!("expansion was cancelled before we asked");
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(1)),
            }
        }
        worker.cancel_expand();

        // The sentinel uses a different index, so a stale `Expanded` from the
        // cancelled part is distinguishable and must never appear.
        worker.expand_with_seq(sentinel.clone(), 1, limits, 9);
        let expanded = settle(&worker, |event| match event {
            Event::Expanded { index, text } => {
                assert_ne!(index, 0, "a cancelled expansion must emit no Expanded");
                Some((index, text))
            }
            Event::ExpandFailed { index, reason } => {
                panic!("expansion {index} must not fail: {reason}");
            }
            Event::Reading { .. }
            | Event::Indexed { .. }
            | Event::IndexFailed { .. }
            | Event::ExpandProgress { .. }
            | Event::ExpandCancelled { .. } => None,
        });
        assert_eq!(expanded.0, 1);
        assert_eq!(expanded.1.text, "two\n");
    }

    #[test]
    fn expansion_reuses_the_cached_text() {
        let bytes = tiny_capture_bytes();
        let limits = CaptureLimits::default();
        let capture = Arc::new(Capture::from_bytes(&bytes, &limits).unwrap());
        let worker = Worker::spawn();

        worker.expand_with_seq(capture.clone(), 0, limits, 0);
        let first = settle(&worker, |event| match event {
            Event::Expanded { index, text } => Some((index, text)),
            Event::ExpandFailed { reason, .. } => panic!("expansion must succeed: {reason}"),
            Event::Reading { .. }
            | Event::Indexed { .. }
            | Event::IndexFailed { .. }
            | Event::ExpandProgress { .. }
            | Event::ExpandCancelled { .. } => None,
        });
        assert_eq!(first.0, 0);
        assert_eq!(first.1.text, "hello\n");
        let after_first = worker.cache_stats();
        assert_eq!(after_first.misses, 1, "the first read expands");
        assert_eq!(after_first.hits, 0);
        assert_eq!(after_first.entries, 1);

        worker.expand_with_seq(capture.clone(), 0, limits, 0);
        let second = settle(&worker, |event| match event {
            Event::Expanded { index, text } => Some((index, text)),
            Event::ExpandFailed { reason, .. } => panic!("expansion must succeed: {reason}"),
            Event::Reading { .. }
            | Event::Indexed { .. }
            | Event::IndexFailed { .. }
            | Event::ExpandProgress { .. }
            | Event::ExpandCancelled { .. } => None,
        });
        assert_eq!(second, first, "a cache hit returns the same text");
        let after_second = worker.cache_stats();
        assert_eq!(after_second.hits, 1, "the second read is served from cache");
        assert_eq!(after_second.misses, 1, "no second inflation happened");
        assert_eq!(after_second.entries, 1);
    }
}
