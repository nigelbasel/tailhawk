//! The growth scan, off the window thread — `SPEC.md` §11.3, and M4's one unmet criterion.
//!
//! [`crate::follow`] scans a growing file correctly. It scans it **on the thread that also paints**,
//! and that is what this fixes.
//!
//! ## The measurement this exists because of
//!
//! `docs/HANDOFF.md`, 2026-08-13: a writer held at 50 MB/s for 60 s. Tailhawk indexed all
//! 3,145,710,009 bytes and stayed level — but the window's p95 response was **44 ms against a
//! 16.67 ms frame**, about 22 fps. `PLAN.md`'s M4 criterion is "50 MB/s for 60 s **without dropped
//! frames**", and the throughput half was met while that half was not.
//!
//! The cause was not slow scanning. It was that a 30 ms scan tick and a 16.67 ms vsync **cannot both
//! fit on one thread**, and the scan tick had to be 30 ms because an 8 ms one fell 536 MB behind.
//! There is no budget that satisfies both; the work has to move.
//!
//! ## The worker never touches a `LineIndex`
//!
//! **This is the whole design and it is what keeps the change small.** A shared index would need a
//! lock on the *per-frame* path — `Rows::fetch` resolves a row through it every frame — and a
//! writer holding that lock while appending 40,000 lines is the same stall in a different costume.
//!
//! So the worker emits **[`Delta`]s**: the line starts it found, the extent it measured, how far it
//! scanned. The window thread applies them with [`LineIndex::push_line`], which is a branch and an
//! occasional `Vec` push per line and is measured in microseconds for a tick's worth. The index stays
//! single-owner and unlocked, and `rows.rs`, `view.rs` and `paint.rs` are untouched.
//!
//! What crosses the thread boundary is a `Vec<u64>` and an [`Extent`] — data, not a structure.
//!
//! ## The window thread still says *when*
//!
//! The worker does not poll the file itself. It waits on a doorbell, reads a target length the caller
//! published, and scans up to it. Keeping "when" on the window thread is what makes rotation
//! tractable: §5.5b's drain-then-switch needs the scan **finished** before a member is replaced, and
//! a worker that decided its own schedule would have to be raced with instead of asked.
//!
//! ## Two threads, one handle
//!
//! The worker reads through the same [`LogFile`] the viewport does. §5.2 already requires every read
//! to carry its own explicit offset rather than move a shared file pointer, and that is precisely
//! what makes concurrent reads on one handle sound — `file.rs` says so where it declares `Send` and
//! `Sync`. Reopening by path instead would risk landing on a *different file* across a rotation,
//! which is the bug §5.5 exists to avoid.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use crate::file::LogFile;
use crate::follow::{Follow, Poll};
use crate::index::{Extent, LineIndex};
use crate::{Error, Result};

/// One scan's findings, ready for the window thread to fold into its index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delta {
    /// New line starts, in file order. Applied with [`LineIndex::push_line`].
    pub starts: Vec<u64>,
    /// The horizontal extent of what was scanned, to merge into the index's.
    pub extent: Extent,
    /// One past the last byte scanned.
    pub scanned_to: u64,
    /// **The file is shorter than it was.** Truncation or rotation — the worker stops and leaves it
    /// to the caller, which is `rotation.rs`'s business and not this module's.
    pub shrank: bool,
}

impl Delta {
    /// Folds this into an index. The one way a delta is meant to be consumed.
    ///
    /// Deltas **must be applied in the order the worker produced them**, which the channel
    /// guarantees. Out of order they would interleave line starts from different regions and every
    /// anchor after the first would point at the wrong byte.
    pub fn apply(&self, index: &mut LineIndex) -> u64 {
        for start in &self.starts {
            index.push_line(*start);
        }
        index.set_extent(index.extent().merge(self.extent));
        self.starts.len() as u64
    }
}

/// Shared between the caller and the worker. Nothing else crosses.
struct Shared {
    /// How long the caller last saw the file. The worker scans up to this and no further.
    target: AtomicU64,
    /// Set to stop the worker for good.
    stop: AtomicBool,
    /// **Bumped by every [`Scanner::look`], and the reason "is it idle?" is a *pair* of counters
    /// rather than a flag.**
    ///
    /// A single `idle: bool` loses a race that a test found: the caller's `look` clears it, and the
    /// worker's *previous* pass then sets it back to true, so the caller sees "caught up" for a
    /// request the worker has not started. `the_viewport_can_read_while_the_worker_scans` failed
    /// intermittently on exactly that and passed on the retry, which is the shape of a race and not
    /// of a bug in the scan.
    requested: AtomicU64,
    /// The `requested` value the worker had **when it began** the pass it last finished.
    ///
    /// Read at the start of a pass and stored at the end, so a `look` that lands mid-pass leaves
    /// `completed < requested` and the caller keeps waiting — for the pass that will actually cover
    /// the new target, rather than the one that was already running.
    completed: AtomicU64,
}

/// A growth scan running on its own thread.
pub struct Scanner {
    shared: Arc<Shared>,
    /// Deltas, in file order.
    deltas: Receiver<Delta>,
    /// The doorbell. A send means "target moved, go and look".
    bell: Sender<()>,
    handle: Option<std::thread::JoinHandle<Follow>>,
    /// How far the *caller* has applied, which trails the worker's own position.
    applied_to: u64,
}

impl Scanner {
    /// Starts scanning `file` from wherever `follow` left off.
    ///
    /// The [`Follow`] is moved in and comes back out of [`stop`](Self::stop), so a caller that has to
    /// hand the file over — a rotation — gets the scan position back rather than guessing it.
    pub fn start(file: Arc<LogFile>, follow: Follow) -> Result<Self> {
        let applied_to = follow.scanned_to();
        let shared = Arc::new(Shared {
            target: AtomicU64::new(applied_to),
            stop: AtomicBool::new(false),
            requested: AtomicU64::new(0),
            completed: AtomicU64::new(0),
        });
        let (bell, rung) = mpsc::channel();
        let (send_delta, deltas) = mpsc::channel();

        let worker = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("tailhawk-scan".into())
            .spawn(move || run(file, follow, worker, rung, send_delta))
            .map_err(|e| Error(format!("scan worker: {e}")))?;

        Ok(Self {
            shared,
            deltas,
            bell,
            handle: Some(handle),
            applied_to,
        })
    }

    /// Publishes the file's current length and wakes the worker.
    ///
    /// Cheap and idempotent: a target that has not moved still rings, and the worker finds nothing to
    /// do and goes back to waiting.
    pub fn look(&self, len: u64) {
        self.shared.target.store(len, Ordering::Release);
        // Bumped **after** the target, so a worker that observes this request is guaranteed to see
        // the length that came with it.
        self.shared.requested.fetch_add(1, Ordering::AcqRel);
        let _ = self.bell.send(());
    }

    /// Applies every delta waiting, and reports what changed.
    ///
    /// **Bounded by what has arrived, not by time.** The scanning is already done by the time a delta
    /// exists; what is left is `push_line` per line, which for a 50 MB/s tick is on the order of
    /// 50,000 branch-and-maybe-push operations — microseconds, not milliseconds. Budgeting it would
    /// mean carrying an unapplied backlog, and a viewport that is behind the index it holds is worse
    /// than one that is briefly behind the file.
    pub fn collect(&mut self, index: &mut LineIndex) -> Collected {
        let mut collected = Collected::default();
        loop {
            match self.deltas.try_recv() {
                Ok(delta) => {
                    if delta.shrank {
                        collected.shrank = true;
                        break;
                    }
                    collected.lines += delta.apply(index);
                    self.applied_to = delta.scanned_to;
                }
                Err(TryRecvError::Empty) => break,
                // The worker is gone. Not an error here — `stop` is the ordinary way that happens,
                // and a caller mid-rotation has already taken the `Follow` back.
                Err(TryRecvError::Disconnected) => {
                    collected.finished = true;
                    break;
                }
            }
        }
        collected.applied_to = self.applied_to;
        collected
    }

    /// Whether the worker has finished a pass that began **at or after** the last [`look`](Self::look).
    pub fn caught_up(&self) -> bool {
        self.shared.completed.load(Ordering::Acquire)
            >= self.shared.requested.load(Ordering::Acquire)
    }

    /// Reads to `len` and applies everything, **blocking until the worker is idle**.
    ///
    /// §5.5's drain-then-switch, which is the one place blocking the window thread is correct: a
    /// member about to be replaced must be read to EOF first, and "this is where naive tools lose the
    /// last KB". The wait is bounded by the file's remaining tail, and a rotation is rare.
    pub fn drain(&mut self, index: &mut LineIndex, len: u64) -> Collected {
        self.look(len);
        let mut total = Collected::default();
        // A spin with a yield rather than a condvar: the wait is milliseconds in the ordinary case,
        // this runs once per rotation, and a condvar would put a second synchronisation primitive
        // into a module whose whole claim is that the per-frame path has none.
        //
        // **Deliberately unbounded**, and the alternative is worse. §5.5's requirement is that the
        // old handle is read to EOF before a switch — "this is where naive tools lose the last KB" —
        // so a deadline that fired would lose exactly the bytes this exists to keep. It terminates
        // because the worker always stores `completed` at the end of a pass and a dead worker shows
        // up as `finished`; the scan runs at gigabytes a second, so a pathological wait means a
        // pathological tail.
        loop {
            let step = self.collect(index);
            total.lines += step.lines;
            total.shrank |= step.shrank;
            total.finished |= step.finished;
            total.applied_to = step.applied_to;
            // `caught_up` is checked **after** collecting, so the deltas of the pass that just
            // finished are already in the index when it reports true.
            if step.finished || step.shrank || self.caught_up() {
                break;
            }
            std::thread::yield_now();
        }
        // One more sweep: the worker can publish its last delta and then `completed` between the
        // `collect` above and the `caught_up` that let us out.
        let last = self.collect(index);
        total.lines += last.lines;
        total.shrank |= last.shrank;
        total.finished |= last.finished;
        if last.applied_to != 0 {
            total.applied_to = last.applied_to;
        }
        total
    }

    /// Stops the worker and takes the [`Follow`] back, so a caller can hand the file on.
    ///
    /// **Does not drain.** [`drain`](Self::drain) is a separate call for the same reason
    /// `rotation.rs` keeps `check` and `open_current` apart: a caller must not be able to switch away
    /// from a file without having decided to read the rest of it first.
    pub fn stop(mut self) -> Option<Follow> {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.bell.send(());
        self.handle.take().and_then(|h| h.join().ok())
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.bell.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// What one [`Scanner::collect`] folded in.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Collected {
    pub lines: u64,
    /// The file shrank — truncation or rotation. The caller decides; see `rotation.rs`.
    pub shrank: bool,
    /// The worker has exited.
    pub finished: bool,
    /// One past the last byte whose lines are now in the index.
    pub applied_to: u64,
}

/// The worker loop. Returns the [`Follow`] so [`Scanner::stop`] can hand the position back.
fn run(
    file: Arc<LogFile>,
    mut follow: Follow,
    shared: Arc<Shared>,
    bell: Receiver<()>,
    deltas: Sender<Delta>,
) -> Follow {
    while bell.recv().is_ok() {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        // Read **before** the pass. Storing this at the end is what makes `caught_up` mean "a pass
        // that began after your `look` has finished" rather than "some pass has finished".
        let serving = shared.requested.load(Ordering::Acquire);
        loop {
            if shared.stop.load(Ordering::Acquire) {
                return follow;
            }
            let target = shared.target.load(Ordering::Acquire);
            let mut collector = Collector::default();
            let outcome = follow.poll(&*file, &mut collector, target);
            let more = match outcome {
                Ok(Poll::Grew { more, .. }) => more,
                Ok(Poll::Shrank { .. }) => {
                    let _ = deltas.send(Delta {
                        shrank: true,
                        scanned_to: follow.scanned_to(),
                        ..Delta::default()
                    });
                    break;
                }
                Ok(Poll::Unchanged) => break,
                // A read failed. Stopping is right: the caller's next rotation check sees whatever
                // happened to the file, and inventing lines from a failed read would be worse than
                // a viewport that is briefly behind.
                Err(_) => break,
            };
            collector.scanned_to = follow.scanned_to();
            if deltas.send(collector.into_delta()).is_err() {
                return follow;
            }
            if !more {
                break;
            }
        }
        shared.completed.store(serving, Ordering::Release);
    }
    follow
}

/// Stands in for the [`LineIndex`] `Follow::poll` would otherwise append to.
///
/// **This is the seam that lets the scan run off-thread without a lock.** `Follow` needs somewhere to
/// put line starts and something to pop when a trailing terminator turns out not to open a line yet;
/// it does not need those things to be an index. Collecting into a `Vec` gives identical behaviour —
/// the phantom-line pop is a `Vec::pop` — and the result is sendable.
#[derive(Default)]
struct Collector {
    starts: Vec<u64>,
    extent: Extent,
    scanned_to: u64,
}

impl crate::follow::LineSink for Collector {
    fn push_line(&mut self, offset: u64) {
        self.starts.push(offset);
    }

    /// The phantom line, taken back. A `Vec::pop` here is the exact counterpart of
    /// `LineIndex::pop_line`, and it is only ever called for a line this same scan pushed.
    fn pop_line(&mut self) {
        self.starts.pop();
    }

    fn merge_extent(&mut self, extent: Extent) {
        self.extent = self.extent.merge(extent);
    }
}

impl Collector {
    fn into_delta(self) -> Delta {
        Delta {
            starts: self.starts,
            extent: self.extent,
            scanned_to: self.scanned_to,
            shrank: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Charset;
    use crate::indexer::{build_index, IndexOptions};
    use std::io::Write;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("tailhawk-scanner");
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        path
    }

    /// Opens `path`, indexes what is there, and starts a scanner on the rest.
    fn open(path: &std::path::Path) -> (Arc<LogFile>, LineIndex, Scanner) {
        let file = Arc::new(LogFile::open(path).expect("open"));
        let end = file.len().expect("len");
        let index =
            build_index(&*file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("index");
        let follow = Follow::after_build(Charset::UTF_8, &index, end);
        let scanner = Scanner::start(Arc::clone(&file), follow).expect("start");
        (file, index, scanner)
    }

    fn append(path: &std::path::Path, text: &str) {
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .expect("append");
        file.write_all(text.as_bytes()).expect("write");
        file.flush().expect("flush");
    }

    /// The oracle every test compares against: the same bytes indexed in one go.
    fn oracle(path: &std::path::Path) -> LineIndex {
        let file = LogFile::open(path).expect("open");
        let end = file.len().expect("len");
        build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("oracle")
    }

    #[test]
    fn a_worker_scan_and_an_in_thread_scan_reach_the_same_index() {
        let path = scratch("agrees.log");
        std::fs::write(&path, "one\ntwo\n").expect("seed");
        let (file, mut index, mut scanner) = open(&path);

        append(&path, "three\nfour\nfive\n");
        let len = file.len().expect("len");
        let collected = scanner.drain(&mut index, len);

        assert_eq!(collected.lines, 3);
        assert_eq!(index.line_count(), oracle(&path).line_count());
        assert_eq!(collected.applied_to, len);
    }

    /// **The phantom line, across the thread boundary.** `Follow` pops the line a trailing terminator
    /// opens because it is not a line yet, and puts it back when a byte arrives. A `Collector` has to
    /// reproduce that exactly, or every append after the first is one line short — the bug that
    /// `follow.rs` exists to prevent, reintroduced at the seam.
    #[test]
    fn a_line_appended_after_a_trailing_newline_survives_the_thread_boundary() {
        let path = scratch("phantom.log");
        std::fs::write(&path, "one\n").expect("seed");
        let (file, mut index, mut scanner) = open(&path);
        assert_eq!(index.line_count(), 1);

        for text in ["two\n", "three\n", "four\n"] {
            append(&path, text);
            scanner.drain(&mut index, file.len().expect("len"));
        }
        assert_eq!(index.line_count(), 4);
        assert_eq!(index.line_count(), oracle(&path).line_count());
    }

    /// Byte at a time is the split that finds what a comfortable one hides — the same discipline
    /// `lines.rs` records, applied to the thread boundary rather than to a decoder.
    #[test]
    fn appending_one_byte_at_a_time_agrees_with_indexing_the_whole_file() {
        let path = scratch("bytewise.log");
        std::fs::write(&path, "").expect("seed");
        let (file, mut index, mut scanner) = open(&path);

        let text = "alpha\nbeta\r\n\ngamma\r\n";
        for byte in text.bytes() {
            append(&path, &(byte as char).to_string());
            scanner.drain(&mut index, file.len().expect("len"));
        }
        assert_eq!(index.line_count(), oracle(&path).line_count());
    }

    /// §5.5's truncation, seen from the worker. It reports and stops rather than guessing, because
    /// guessing wrong loses bytes — and the caller is the one holding `rotation.rs`.
    #[test]
    fn a_shrinking_file_is_reported_rather_than_scanned() {
        let path = scratch("shrank.log");
        std::fs::write(&path, "one\ntwo\nthree\n").expect("seed");
        let (_file, mut index, mut scanner) = open(&path);

        std::fs::write(&path, "x\n").expect("truncate");
        let collected = scanner.drain(&mut index, 2);
        assert!(collected.shrank);
    }

    /// The worker hands the scan position back, so a rotation can carry on from exactly where the
    /// old file was left rather than from a number the caller remembered.
    #[test]
    fn stopping_returns_the_follow_position() {
        let path = scratch("handback.log");
        std::fs::write(&path, "one\n").expect("seed");
        let (file, mut index, mut scanner) = open(&path);
        append(&path, "two\nthree\n");
        let len = file.len().expect("len");
        scanner.drain(&mut index, len);

        let follow = scanner.stop().expect("the worker should hand it back");
        assert_eq!(follow.scanned_to(), len);
    }

    /// Nothing to do must be cheap and quiet, because this is rung on every timer tick of a file
    /// nobody is writing to.
    #[test]
    fn a_look_at_an_unchanged_file_produces_nothing() {
        let path = scratch("quiet.log");
        std::fs::write(&path, "one\ntwo\n").expect("seed");
        let (file, mut index, mut scanner) = open(&path);
        let before = index.line_count();

        for _ in 0..5 {
            scanner.drain(&mut index, file.len().expect("len"));
        }
        assert_eq!(index.line_count(), before);
    }

    /// **Two threads reading one handle**, which §5.2's positional reads are what make sound. If the
    /// viewport's reads and the worker's shared a file pointer this would return interleaved
    /// nonsense, and it is the assumption the whole module rests on.
    #[test]
    fn the_viewport_can_read_while_the_worker_scans() {
        let path = scratch("concurrent.log");
        let body: String = (0..20_000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, "seed\n").expect("seed");
        let (file, mut index, mut scanner) = open(&path);
        append(&path, &body);
        scanner.look(file.len().expect("len"));

        // Read from the front while the worker is scanning the rest.
        let mut buf = [0u8; 5];
        for _ in 0..200 {
            let read = file.read_at(0, &mut buf).expect("read");
            assert_eq!(&buf[..read], b"seed\n");
        }
        scanner.drain(&mut index, file.len().expect("len"));
        assert_eq!(index.line_count(), oracle(&path).line_count());
    }
}
