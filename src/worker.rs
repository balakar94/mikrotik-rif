//! Background jobs so reading, indexing and expansion never block the window.
//!
//! A capture can be hundreds of megabytes, so the file read, the index pass and
//! the zlib expansion all run on a dedicated thread. The interface thread only
//! sends jobs and drains events once per frame. The read phase reports real
//! byte progress so the opening animation is driven by actual work.
//!
//! Wave2 notes: [`Event`] shapes are frozen on purpose — `app.rs` matches them
//! exhaustively, so new data (expansion sequences, error kinds) travels through
//! new methods and variants only once the interface is updated to match them.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::parser::{Capture, CaptureLimits, PartText};

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
    ///
    /// Wave2 will add the request `seq` here so the interface can drop stale
    /// arrivals itself; that also requires updating the exhaustive match in
    /// `app.rs`, which is why the echo stays deferred.
    Expanded {
        /// Index of the expanded part.
        index: usize,
        /// Expanded text.
        text: PartText,
    },
    /// One part could not be expanded.
    ///
    /// Same Wave2 note as [`Event::Expanded`]: the `seq` echo stays deferred
    /// until the interface match is updated alongside.
    ExpandFailed {
        /// Index of the part that failed.
        index: usize,
        /// Human-readable reason.
        reason: String,
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
    // Wave2 staging: the interface does not send this yet, so the variant is
    // only constructed by `Worker::cancel` and the unit tests below.
    #[allow(dead_code)]
    Cancel,
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
}

impl Worker {
    /// Start the worker thread.
    #[must_use]
    pub fn spawn() -> Self {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (events_tx, events_rx) = mpsc::channel::<Event>();
        let index_epoch = Arc::new(AtomicU64::new(0));
        let latest_expand_seq = Arc::new(AtomicU64::new(0));
        let index_epoch_thread = Arc::clone(&index_epoch);
        let latest_expand_seq_thread = Arc::clone(&latest_expand_seq);

        thread::Builder::new()
            .name("mikrotik-rif-worker".to_owned())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let keep_going = match job {
                        Job::Index { path, limits } => {
                            let epoch = index_epoch_thread.load(Ordering::SeqCst);
                            index_with_progress(
                                &path,
                                &limits,
                                &events_tx,
                                epoch,
                                &index_epoch_thread,
                            )
                        }
                        Job::Expand {
                            capture,
                            index,
                            limits,
                            seq,
                        } => match expand(&capture, index, &limits, seq, &latest_expand_seq_thread)
                        {
                            Some(event) => events_tx.send(event).is_ok(),
                            None => true,
                        },
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

    /// Ask the worker to abandon the in-flight index pass.
    ///
    /// Preemptive: bumps this worker's index generation immediately, so the
    /// in-flight pass observes the divergence at its next
    /// [`CANCEL_CHECK_EVERY`]-chunk check and stops quietly, even though the
    /// queued [`Job::Cancel`] itself is only processed after the current job
    /// (the queue is FIFO behind a blocking `recv`). The queued job bumps the
    /// generation a second time to drain any index already waiting behind the
    /// in-flight one; a later `index` then starts a fresh generation. Base
    /// plumbing only — the interface does not call this yet (Wave2 wiring).
    #[allow(dead_code)]
    pub fn cancel(&self) {
        self.index_epoch.fetch_add(1, Ordering::SeqCst);
        let _ = self.jobs.send(Job::Cancel);
    }

    /// Take the next event, if one is waiting.
    #[must_use]
    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
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
/// (checked every [`CANCEL_CHECK_EVERY`] read chunks). Progress events go out
/// at most once per [`PROGRESS_STEP`] bytes plus one final report, so
/// `reported` only moves when an event is really sent.
fn index_with_progress(
    path: &PathBuf,
    limits: &CaptureLimits,
    events: &Sender<Event>,
    epoch: u64,
    index_epoch: &AtomicU64,
) -> bool {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            return events
                .send(Event::IndexFailed {
                    path: path.clone(),
                    reason: format!("cannot read the file: {error}"),
                })
                .is_ok();
        }
    };

    let meta_len = file.metadata().ok().map(|meta| meta.len());
    if meta_len.is_some_and(|len| !size_allowed(len)) {
        return events
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
        if chunks.is_multiple_of(CANCEL_CHECK_EVERY) && index_epoch.load(Ordering::SeqCst) != epoch
        {
            return true;
        }
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                received += read as u64;
                if !size_allowed(received) {
                    return events
                        .send(Event::IndexFailed {
                            path: path.clone(),
                            reason: TOO_LARGE_REASON.to_owned(),
                        })
                        .is_ok();
                }
                if received - reported >= PROGRESS_STEP {
                    reported = received;
                    if events.send(Event::Reading { received, total }).is_err() {
                        return false;
                    }
                }
                chunks += 1;
            }
            Err(error) => {
                return events
                    .send(Event::IndexFailed {
                        path: path.clone(),
                        reason: format!("cannot read the file: {error}"),
                    })
                    .is_ok();
            }
        }
    }

    if events.send(Event::Reading { received, total }).is_err() {
        return false;
    }

    let event = match Capture::from_bytes(&bytes, limits) {
        Ok(capture) => Event::Indexed {
            path: path.clone(),
            capture: Arc::new(capture),
        },
        Err(error) => Event::IndexFailed {
            path: path.clone(),
            reason: error.to_string(),
        },
    };
    events.send(event).is_ok()
}

/// Expand one part, unless `seq` was superseded by a newer request.
///
/// Returns `None` for a stale request so the caller emits no event; the newer
/// request still queued behind settles the interface instead. Sequence `0` is
/// the legacy path and is always decoded. `latest` is the owning [`Worker`]'s
/// highest sequence seen so far.
fn expand(
    capture: &Capture,
    index: usize,
    limits: &CaptureLimits,
    seq: u64,
    latest: &AtomicU64,
) -> Option<Event> {
    if seq != 0 && seq < latest.load(Ordering::SeqCst) {
        return None;
    }
    Some(match capture.read(index, limits) {
        Ok(text) => Event::Expanded { index, text },
        Err(error) => Event::ExpandFailed {
            index,
            reason: error.to_string(),
        },
    })
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

    /// Smallest well-formed capture: one part holding `hello`.
    fn tiny_capture_bytes() -> Vec<u8> {
        let mut plain = b"export".to_vec();
        plain.push(0);
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(b"hello\n").unwrap();
        plain.extend_from_slice(&encoder.finish().unwrap());

        let mut source = Vec::new();
        source.extend_from_slice(OPEN_MARKER);
        source.push(b'\n');
        source.extend_from_slice(&codec::pack(&plain));
        source.push(b'\n');
        source.extend_from_slice(CLOSE_MARKER);
        source.push(b'\n');
        source
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
            Event::Reading { .. } | Event::Expanded { .. } | Event::ExpandFailed { .. } => None,
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
            Event::Reading { .. } | Event::Indexed { .. } | Event::IndexFailed { .. } => None,
        });
        assert_eq!(first.0, 0);
        assert_eq!(first.1.text, "hello\n");

        std::thread::sleep(Duration::from_millis(500));
        assert!(
            worker.poll().is_none(),
            "the superseded request must emit no event"
        );
    }
}
