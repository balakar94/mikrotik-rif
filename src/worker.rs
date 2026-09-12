//! Background jobs so reading, indexing and expansion never block the window.
//!
//! A capture can be hundreds of megabytes, so the file read, the index pass and
//! the zlib expansion all run on a dedicated thread. The interface thread only
//! sends jobs and drains events once per frame. The read phase reports real
//! byte progress so the opening animation is driven by actual work.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::parser::{Capture, CaptureLimits, PartText};

/// Chunk size used while streaming the file from disk.
const READ_CHUNK: usize = 256 * 1024;

/// Progress is reported at most once per this many bytes.
const PROGRESS_STEP: u64 = 1024 * 1024;

/// Cap on the upfront buffer reservation, so a huge file cannot over-allocate.
const RESERVE_CAP: u64 = 64 * 1024 * 1024;

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
}

enum Job {
    Index(PathBuf),
    Expand {
        capture: Arc<Capture>,
        index: usize,
        limits: CaptureLimits,
    },
}

/// Handle to the single worker thread that processes jobs in order.
pub struct Worker {
    jobs: Sender<Job>,
    events: Receiver<Event>,
}

impl Worker {
    /// Start the worker thread.
    #[must_use]
    pub fn spawn() -> Self {
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let (events_tx, events_rx) = mpsc::channel::<Event>();

        thread::Builder::new()
            .name("mikrotik-rif-worker".to_owned())
            .spawn(move || {
                while let Ok(job) = jobs_rx.recv() {
                    let keep_going = match job {
                        Job::Index(path) => index_with_progress(&path, &events_tx),
                        Job::Expand {
                            capture,
                            index,
                            limits,
                        } => {
                            let event = expand(&capture, index, &limits);
                            events_tx.send(event).is_ok()
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
        }
    }

    /// Queue a capture to be read and indexed.
    pub fn index(&self, path: PathBuf) {
        let _ = self.jobs.send(Job::Index(path));
    }

    /// Queue a part to be expanded.
    pub fn expand(&self, capture: Arc<Capture>, index: usize, limits: CaptureLimits) {
        let _ = self.jobs.send(Job::Expand {
            capture,
            index,
            limits,
        });
    }

    /// Take the next event, if one is waiting.
    #[must_use]
    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

/// Stream the file from disk, reporting progress, then index it.
fn index_with_progress(path: &PathBuf, events: &Sender<Event>) -> bool {
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

    let total = file
        .metadata()
        .ok()
        .map(|meta| meta.len())
        .filter(|size| *size > 0);
    let reserve = total.unwrap_or(0).min(RESERVE_CAP);
    let mut bytes: Vec<u8> = Vec::with_capacity(usize::try_from(reserve).unwrap_or(0));
    let mut chunk = vec![0u8; READ_CHUNK];
    let mut received: u64 = 0;
    let mut reported: u64 = 0;

    loop {
        match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                bytes.extend_from_slice(&chunk[..read]);
                received += read as u64;
                if received - reported >= PROGRESS_STEP
                    && events.send(Event::Reading { received, total }).is_err()
                {
                    return false;
                }
                reported = received;
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

    let event = match Capture::from_bytes(&bytes, &CaptureLimits::default()) {
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

fn expand(capture: &Capture, index: usize, limits: &CaptureLimits) -> Event {
    match capture.read(index, limits) {
        Ok(text) => Event::Expanded { index, text },
        Err(error) => Event::ExpandFailed {
            index,
            reason: error.to_string(),
        },
    }
}
