//! The Loki tail — `LOKI.md` §5, and the standing instruction in `CLAUDE.md`.
//!
//! **What `Open remote source` did before this was a fetch, not a tail.** One `query_range` for the
//! last hour, written to a spill, opened as a document. The window said *"● following"* and it was
//! telling the truth about the file — nothing ever wrote to that file again, so there was nothing
//! to follow. The owner noticed before any test did.
//!
//! This is the loop that makes it a tail: ask Loki for everything after the newest record already
//! held, append it to the same spill, and let the follow machinery notice the file grew. The UI
//! thread does nothing new — following a growing file is a thing it already does, for the stdin
//! pump, with tests.
//!
//! **The decisions are pure and tested; the thread is not.** [`window_after`] and [`Backoff`] are
//! this module's whole judgement and neither needs a network, a window or a clock it does not own.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use tailhawk_core::loki::{Direction, Nanos, Window};
use tailhawk_core::settings::Source;

/// How long between polls when everything is working.
///
/// `LOKI.md` §5 wants a tail that feels live without asking a shared server a question every
/// frame. Five seconds is the compromise: a person watching a deploy sees it, and a window left
/// open all day costs the estate 720 queries rather than 86,400.
pub const POLL: Duration = Duration::from_secs(5);

/// How many records one poll asks for.
///
/// `loki::MAX_LIMIT` is 5,000 and this is deliberately under it: a poll is a delta, and a request
/// that asks for everything a busy estate can produce is a request that takes long enough to make
/// the *next* delta large. A burst bigger than this is not lost — [`Direction::Forward`] leaves the
/// newest behind and [`caught_up`] goes straight round again instead of sleeping.
pub const TAIL_LIMIT: u32 = 2_000;

/// Whether a poll that returned `records` has caught up with the source.
///
/// **A full answer means there is more**, and sleeping on one is how a tail falls permanently
/// behind. Measured against the real estate: the first tail ran at exactly [`TAIL_LIMIT`] records
/// every poll and was eighty-four seconds behind after a minute, losing ground the whole time,
/// because production writes faster than one poll's worth per interval. A tail that is behind
/// should be asking again, not waiting.
///
/// It cannot spin: every round advances the mark past what it just wrote, so a tail that is behind
/// is doing work, and one that is level gets a short answer and sleeps.
pub fn caught_up(records: usize) -> bool {
    (records as u64) < TAIL_LIMIT as u64
}

/// How many recent spill lines are remembered for de-duplication.
///
/// Bounded by count rather than by time because a count is what a ring can hold without a clock:
/// two polls' worth at a few hundred records a second, which covers everything an
/// [`OVERLAP_NANOS`] window can bring back twice. A burst larger than this in one overlap would let
/// a repeat through, and that is preferred to a set that grows for the life of the window.
pub const RECENT_LINES: usize = 8_192;

/// The lines most recently written to the spill, so a record the overlap fetches again is written
/// once. **Keyed by the line itself, hashed** — `clef_line` is deterministic, so the same record
/// spills to the same bytes — and evicted oldest-first at [`RECENT_LINES`].
#[derive(Debug, Default)]
pub struct Recent {
    ring: std::collections::VecDeque<u64>,
    seen: std::collections::HashSet<u64>,
    cap: usize,
}

impl Recent {
    pub fn new(cap: usize) -> Recent {
        Recent {
            ring: std::collections::VecDeque::with_capacity(cap.min(RECENT_LINES)),
            seen: std::collections::HashSet::with_capacity(cap.min(RECENT_LINES)),
            cap: cap.max(1),
        }
    }

    /// Whether `line` is new — and, if so, remembers it. `false` means it has already been written.
    pub fn admit(&mut self, line: &str) -> bool {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        line.hash(&mut h);
        let key = h.finish();
        if !self.seen.insert(key) {
            return false;
        }
        self.ring.push_back(key);
        while self.ring.len() > self.cap {
            if let Some(old) = self.ring.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

/// The window a poll should ask for, given the newest record already held.
///
/// **It starts from the data, not from the clock.** A window of "the last five seconds" drops
/// whatever arrived while the previous request was in flight — and a request that took six seconds
/// loses a second of log with nothing to say so, which is this project's worst failure shape. The
/// newest record already spilled is the only honest place to resume from.
///
/// **Plus one nanosecond**, because `Window::start` is inclusive: resuming at the timestamp itself
/// re-fetches the record that carries it, and the spill would show it twice.
pub fn window_after(since: Nanos, now: Nanos) -> Window {
    // **Start a little before the newest record held, and end a little before now.** A record
    // reaches Loki's index some seconds after its own timestamp — the estate measured around
    // thirty at the worst moment of a busy hour — and a window that starts at the newest record
    // *seen* skips everything indexed late with an earlier stamp, for ever, with nothing to say
    // so. The overlap asks for that stretch again; [`Recent`] keeps the repeats off the spill.
    // The lag holds the window's end back so the newest few seconds are asked for once they
    // have had a chance to land, rather than being crossed off before they exist.
    // `Nanos` is signed, so a saturating subtraction bottoms out far below zero rather than at it;
    // the clamp is what keeps a mark near the epoch from asking Loki for a negative start.
    let start = since.saturating_sub(OVERLAP_NANOS).max(0).saturating_add(1);
    Window {
        start,
        // A clock that has gone backwards — a correction, a VM resuming — must not produce a
        // window that ends before it starts. An empty window is the right answer: nothing is newer
        // than what is already held, which is exactly what the clock is claiming.
        end: now.saturating_sub(LAG_NANOS).max(start),
    }
}

/// How far behind the clock the window ends. A record indexed within this of its own timestamp is
/// never missed; one indexed later is caught by the overlap for as long as the overlap reaches.
pub const LAG_NANOS: Nanos = 5 * 1_000_000_000;

/// How far before the newest record held each window starts again. Sized with [`TAIL_LIMIT`] in
/// mind: at a few hundred records a second this re-fetches a few hundred, which the limit absorbs;
/// a minute would not, and a tail whose every poll fills its limit with repeats never catches up.
pub const OVERLAP_NANOS: Nanos = 5 * 1_000_000_000;

/// How long to wait after a poll that failed.
///
/// **An unreachable environment must not fill the status bar.** Repeating the same error every five
/// seconds tells a reader nothing they did not know after the first one, and buries whatever else
/// the bar was saying. The wait doubles up to a ceiling, and a single success clears it, so a
/// source that recovers catches up without being reopened.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    /// The longest a tail will wait between attempts, however long it has been failing. A source
    /// that comes back after an hour should be noticed in under a minute, not in another hour.
    pub const CEILING: Duration = Duration::from_secs(60);

    /// How long to wait before the next attempt.
    pub fn wait(&self) -> Duration {
        match self.failures {
            0 => POLL,
            n => POLL
                .saturating_mul(1u32.checked_shl(n.min(16)).unwrap_or(u32::MAX))
                .min(Backoff::CEILING),
        }
    }

    /// Whether this failure is the one worth telling the reader about: the first of a run.
    pub fn should_say(&self) -> bool {
        self.failures == 1
    }

    pub fn failed(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    pub fn succeeded(&mut self) {
        self.failures = 0;
    }
}

/// A running tail. Dropping it stops the thread.
pub struct Tail {
    stop: Arc<AtomicBool>,
}

impl Tail {
    /// Starts tailing `source` into the spill at `path`, resuming after `since`.
    ///
    /// Faults are sent on `notices` rather than shown from the worker: the status bar belongs to
    /// the UI thread, and a background thread reaching into it is how a repaint ends up on the
    /// wrong side of a `RefCell` borrow.
    pub fn start(
        source: Source,
        path: PathBuf,
        since: Nanos,
        seed: &str,
        notices: Sender<String>,
    ) -> Tail {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let name = source.name.clone();
        // The opening pull's lines are what the first overlap would fetch again; seeded here so the
        // first poll after opening writes nothing twice.
        let mut recent = Recent::new(RECENT_LINES);
        for line in seed.lines() {
            recent.admit(line);
        }
        std::thread::spawn(move || {
            let mut since = since;
            let mut backoff = Backoff::default();
            let mut behind = false;
            loop {
                // A tail that is behind asks again at once; only one that is level waits.
                let wait = if behind {
                    Duration::ZERO
                } else {
                    backoff.wait()
                };
                if !nap(&flag, wait) {
                    return;
                }
                let now = match now_nanos() {
                    Some(now) => now,
                    None => continue,
                };
                match crate::pull::pull(
                    &source,
                    window_after(since, now),
                    TAIL_LIMIT,
                    Direction::Forward,
                ) {
                    Ok(pulled) => {
                        backoff.succeeded();
                        // Behind is judged on what Loki returned, repeats included: the limit
                        // was spent on them all.
                        behind = !caught_up(pulled.records);
                        if pulled.records == 0 {
                            continue;
                        }
                        // The overlap asks for the last few seconds again on purpose; only the
                        // lines the spill has not seen are written.
                        let mut fresh = String::with_capacity(pulled.clef.len());
                        for line in pulled.clef.lines() {
                            if recent.admit(line) {
                                fresh.push_str(line);
                                fresh.push('\n');
                            }
                        }
                        if fresh.is_empty() {
                            continue;
                        }
                        if append(&path, &fresh).is_err() {
                            let _ = notices.send(format!("{name}: could not write new records"));
                            return;
                        }
                        // **Only what was actually written moves the mark.** Advancing on the
                        // window's end instead would skip whatever Loki had not yet indexed at the
                        // moment of the query — records that exist, are late, and would never be
                        // asked for again.
                        if let Some(newest) = pulled.newest {
                            since = since.max(newest);
                        }
                    }
                    Err(why) => {
                        // A failure is not a reason to hurry: whatever the last poll said about
                        // being behind, the next attempt waits.
                        behind = false;
                        backoff.failed();
                        if backoff.should_say() {
                            let _ = notices.send(format!("{name}: {why}"));
                        }
                    }
                }
            }
        });
        Tail { stop }
    }
}

impl Drop for Tail {
    /// Asks the thread to stop and does **not** wait for it. A join here would block the UI thread
    /// for as long as a request in flight takes, which on a source that has gone away is the
    /// transport's whole timeout — a window that will not close because a server will not answer.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Sleeps in slices, returning `false` as soon as the tail is asked to stop.
///
/// A single `sleep` of the backoff's ceiling would keep a thread alive a minute past the window
/// closing. The slice is the granularity at which stopping is noticed.
fn nap(stop: &AtomicBool, total: Duration) -> bool {
    const SLICE: Duration = Duration::from_millis(100);
    let mut left = total;
    while !left.is_zero() {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let step = left.min(SLICE);
        std::thread::sleep(step);
        left -= step;
    }
    !stop.load(Ordering::Relaxed)
}

fn now_nanos() -> Option<Nanos> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_nanos() as Nanos)
}

fn append(path: &PathBuf, clef: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(clef.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The window starts before the newest record held and ends before now.** Records reach
    /// Loki's index seconds after their own timestamps, so a window that started at the newest
    /// record *seen* skipped everything indexed late with an earlier stamp — for ever, silently.
    /// The overlap re-asks that stretch and [`Recent`] keeps the repeats off the spill; the lag
    /// holds the end back so the newest seconds are asked for once they have had a chance to land.
    #[test]
    fn the_next_window_overlaps_the_last_and_lags_the_clock() {
        let since = 100 * 1_000_000_000;
        let now = 200 * 1_000_000_000;
        let w = window_after(since, now);
        assert_eq!(
            w.start,
            since - OVERLAP_NANOS + 1,
            "one past the overlap's start"
        );
        assert_eq!(w.end, now - LAG_NANOS, "held back by the lag");
        assert!(w.start < w.end);
    }

    /// A mark near the epoch cannot overlap into negative time.
    #[test]
    fn an_overlap_before_the_epoch_is_clamped() {
        let w = window_after(1_000, 200 * 1_000_000_000);
        assert_eq!(w.start, 1, "saturated at zero, then one past it");
    }

    /// **A clock that has gone backwards must not make a window that ends before it starts.** A
    /// correction or a resumed VM is enough; what a negative window does at the server is not
    /// something to find out from a user's log being wrong.
    #[test]
    fn a_clock_that_went_backwards_yields_an_empty_window_not_a_reversed_one() {
        let since = 900 * 1_000_000_000;
        let w = window_after(since, 100 * 1_000_000_000);
        assert!(w.end >= w.start, "{w:?}");
        assert_eq!(w.end, w.start, "empty, not reversed");
    }

    /// A line seen once is written once; a new line is admitted; the ring forgets oldest-first.
    #[test]
    fn a_repeated_line_is_refused_and_the_ring_forgets_the_oldest() {
        let mut r = Recent::new(3);
        assert!(r.admit("a"));
        assert!(
            !r.admit("a"),
            "the overlap brought it back; it is not written twice"
        );
        assert!(r.admit("b"));
        assert!(r.admit("c"));
        assert!(r.admit("d"), "a fourth line evicts the first");
        assert!(
            r.admit("a"),
            "and the first is new again — bounded by count, not for ever"
        );
        assert!(!r.admit("d"));
    }

    /// The seed is the opening pull, so the very first poll's overlap writes nothing twice.
    #[test]
    fn a_seeded_ring_refuses_the_opening_pulls_lines() {
        let opening = "{\"@t\":\"a\",\"@m\":\"one\"}\n{\"@t\":\"b\",\"@m\":\"two\"}\n";
        let mut r = Recent::new(RECENT_LINES);
        for line in opening.lines() {
            r.admit(line);
        }
        assert!(!r.admit("{\"@t\":\"b\",\"@m\":\"two\"}"));
        assert!(r.admit("{\"@t\":\"c\",\"@m\":\"three\"}"));
    }

    /// A healthy tail polls at the interval; a failing one waits longer each time, up to a ceiling
    /// that keeps a recovered source from staying unnoticed.
    #[test]
    fn backoff_doubles_to_a_ceiling_and_a_success_clears_it() {
        let mut b = Backoff::default();
        assert_eq!(b.wait(), POLL);
        b.failed();
        let first = b.wait();
        b.failed();
        assert!(b.wait() > first, "{:?} should exceed {first:?}", b.wait());
        for _ in 0..40 {
            b.failed();
        }
        assert_eq!(b.wait(), Backoff::CEILING, "it must not grow without bound");
        b.succeeded();
        assert_eq!(b.wait(), POLL, "one success is enough to catch up promptly");
    }

    /// **The reader is told once, not every five seconds.** An environment that is down would
    /// otherwise overwrite whatever else the status bar was saying, for as long as it stayed down.
    #[test]
    fn only_the_first_failure_of_a_run_is_said() {
        let mut b = Backoff::default();
        assert!(!b.should_say(), "nothing has failed yet");
        b.failed();
        assert!(b.should_say());
        b.failed();
        assert!(!b.should_say(), "the second is the same news as the first");
        b.succeeded();
        b.failed();
        assert!(b.should_say(), "a fresh outage is fresh news");
    }
}
