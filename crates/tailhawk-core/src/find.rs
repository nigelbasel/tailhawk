//! A search running on a worker, over a snapshot of the log — `SPEC.md` §7.3 and §7.4.
//!
//! [`crate::search`] is the pass itself: two engines, chunking by line, streaming its findings. What
//! it does not say is *where* that pass runs, and for a viewer the answer is the whole feature.
//! §7.4's own measurement is 9.93 s for a full pass over 10 GB; run on the window thread that is ten
//! seconds of frozen application, and §11.3 forbids blocking a frame for a single millisecond.
//!
//! ## A snapshot, not a borrow — and the two things that makes true
//!
//! The worker takes an [`Excerpt`] per member: the file **handle**, shared; the line index,
//! **cloned**; the charset and the member's first row, copied. Nothing it holds can move under it
//! and nothing the window thread does needs a lock.
//!
//! - **The handle is shared rather than reopened.** `set.rs` already keeps it in an `Arc` for the
//!   scanner, for the reason §5.2 gives: positional reads make concurrent reads on one handle sound,
//!   and reopening by path across a rotation risks landing on a different file — which for a search
//!   would mean an index describing bytes the reads no longer return.
//! - **The index is cloned rather than shared.** A followed file's index grows while the search
//!   runs, and a search that sees it grow would report line numbers against a row space that has
//!   moved. So a search covers the log **as it was when the search started**, the count it reports is
//!   against that, and new lines are found by searching again. The clone is ~6.3 MB at 50M lines
//!   (§5.3); see [`LineIndex`](crate::index::LineIndex).
//!
//! ## One search to the user, several to the engine
//!
//! §5.5b makes a set of rolled files one log with one row space, so a search covers the set and
//! reports **set-wide rows**. It cannot be one [`Search`], because members detect their encodings
//! separately (§5.6) and §7.4's engine choice is per charset — a pattern compiled for a UTF-8 member
//! finds nothing in a UTF-16 one, silently. So there is one `Search` per member, sharing one
//! [`Cancel`] and one match budget, and the mapping to set-wide rows happens here where `first_row`
//! is known.
//!
//! Members are searched **oldest first**, matching the row space rather than the likely interest.
//! Ordering by interest — newest first — would make the streamed matches arrive in an order that
//! contradicts the row numbers they carry, and §7.4 already says results are unordered until sorted.
//!
//! ## ⚠ What this does not do
//!
//! - **No debounce.** §7.3 wants the pass restarted on a 300 ms debounce as the user types; this
//!   starts when the user asks it to. The cancellation half of that machinery is here and working —
//!   the debounce is a UI timer, and it belongs with the find bar rather than in the core.
//! - **No progress against a total.** [`Found::scanned`] climbs, and the caller knows the row count,
//!   so a percentage is available to anyone who wants one; nothing here computes it.
//! - **Cross-file search** (§7.4's grouped results across *open sources*) is not this. This is one
//!   document's set of members; a second tab is a second search.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use crate::encoding::Charset;
use crate::file::LogFile;
use crate::index::LineIndex;
use crate::search::{Cancel, Found, Match, Pattern, Search, SearchOptions};
use crate::Result;

/// One member of a set, in the form a worker can search without touching the document.
pub struct Excerpt {
    /// Shared with the window thread. §5.2's positional reads are what make that sound.
    pub file: Arc<LogFile>,
    pub charset: Charset,
    /// A clone taken when the search started. See the module note.
    pub index: LineIndex,
    /// The set-wide row number of this member's line 0, which is what turns a member-local match
    /// into one the viewport can scroll to.
    pub first_row: u64,
}

/// How a pass ended. **Every ending is named**, because "no more matches are arriving" means four
/// different things to a user and only one of them is "that is all of them".
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The whole snapshot was examined.
    Complete,
    /// [`Cancel`] fired — a new search, or the user pressing `Esc`.
    Cancelled,
    /// [`SearchOptions::max_matches`] was reached, so there are matches that were not reported.
    Capped,
    /// A read failed. The matches already sent stand; the rest of the pass did not happen.
    Failed(String),
}

/// What a running search reports as it goes. §7.4: "results stream".
#[derive(Debug)]
pub enum Update {
    /// One chunk's findings, **in set-wide rows**. Chunks arrive out of order; see [`Search::run`].
    Chunk(Found),
    /// The last message on the channel.
    Finished(Outcome),
}

/// A search in flight.
///
/// Dropping it stops the worker: the flag is set and the thread is joined, so no search outlives the
/// document it was started from and no handle is read after the shell has moved on.
pub struct Running {
    cancel: Cancel,
    updates: Receiver<Update>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// What the user typed, kept so a caller can label the results without holding it separately.
    query: String,
}

impl Running {
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Everything reported since the last call. **Never blocks** — a caller on the window thread is
    /// draining this from a timer, and waiting for a chunk is the stall this module exists to avoid.
    pub fn drain(&self) -> impl Iterator<Item = Update> + '_ {
        self.updates.try_iter()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(worker) = self.worker.take() {
            // **Joined rather than detached**, so a closing window cannot leave a thread reading
            // through a handle the document is about to drop. The wait is bounded by one chunk: the
            // flag is checked between chunks and before every read inside one.
            let _ = worker.join();
        }
    }
}

/// Compiles `query` for every member and starts the pass.
///
/// **The compile happens here, on the caller's thread, and its failure is returned rather than
/// streamed.** A pattern the engine will not take is not a search that found nothing — it is a typo,
/// and the user is still looking at the box they typed it into. Reporting it through the same
/// channel as results would put it a timer tick away from the keystroke that caused it.
pub fn start(
    query: &str,
    case_insensitive: bool,
    members: Vec<Excerpt>,
    options: SearchOptions,
) -> Result<Running> {
    // One compile per member, because §7.4's engine choice is per charset. Identical charsets
    // compile identical patterns; the cost is microseconds against a pass measured in seconds, and
    // the alternative is a cache keyed on something that has three possible values.
    let mut work = Vec::with_capacity(members.len());
    for member in members {
        let pattern = Pattern::compile(query, member.charset, case_insensitive)?;
        work.push((member, pattern));
    }

    let cancel = Cancel::new();
    let (tx, updates) = std::sync::mpsc::channel();
    let worker = {
        let cancel = cancel.clone();
        std::thread::Builder::new()
            .name("tailhawk-search".to_owned())
            .spawn(move || run(work, options, &cancel, &tx))
            .map_err(|e| crate::Error(format!("starting the search worker: {e}")))?
    };

    Ok(Running {
        cancel,
        updates,
        worker: Some(worker),
        query: query.to_owned(),
    })
}

/// The worker body: every member in row order, under one budget and one cancel flag.
fn run(
    work: Vec<(Excerpt, Pattern)>,
    options: SearchOptions,
    cancel: &Cancel,
    tx: &Sender<Update>,
) {
    let mut left = options.max_matches;
    let mut outcome = Outcome::Complete;

    for (member, pattern) in work {
        if cancel.cancelled() {
            outcome = Outcome::Cancelled;
            break;
        }
        if left == 0 {
            outcome = Outcome::Capped;
            break;
        }
        let search = Search::sharing(
            pattern,
            member.charset,
            SearchOptions {
                max_matches: left,
                ..options
            },
            cancel,
        );
        // **The rows are shifted here, inside the callback, not by the receiver.** A `Match` that
        // leaves this module already means what its name says everywhere else in the product — the
        // row the viewport scrolls to — so nothing downstream has to remember which member it came
        // from or what to add.
        let sent = search.run(&*member.file, &member.index, |chunk| {
            let _ = tx.send(Update::Chunk(shift(chunk, member.first_row)));
        });
        match sent {
            Ok(found) => left = left.saturating_sub(found.matches.len()),
            Err(e) => {
                outcome = Outcome::Failed(e.to_string());
                break;
            }
        }
        // **Checked after the member as well as before the next one**, or a set of one whose single
        // member fills the budget falls out of the loop reporting `Complete` — a capped count
        // presented as the total, which is the one thing §7.4's streaming results must not do. A
        // pass that found exactly the budget and then genuinely ended is indistinguishable from one
        // that was cut short, and "there may be more" is the safe reading of both.
        if left == 0 {
            outcome = Outcome::Capped;
            break;
        }
    }

    // **Cancellation is asked about last, and it wins over completion.** A pass that was stopped
    // between its last chunk and here has still not examined the rest of the file, and saying
    // "complete" would report a match count as final when it is a partial.
    if cancel.cancelled() {
        outcome = Outcome::Cancelled;
    }
    let _ = tx.send(Update::Finished(outcome));
}

fn shift(chunk: &Found, first_row: u64) -> Found {
    Found {
        matches: chunk
            .matches
            .iter()
            .map(|m| Match {
                line: m.line + first_row,
                ..*m
            })
            .collect(),
        truncated: chunk.truncated,
        scanned: chunk.scanned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::{build_index, IndexOptions};

    /// Collects a whole run, in the order a caller would.
    fn collect(running: &Running) -> (Vec<Match>, u64, Outcome) {
        let mut matches = Vec::new();
        let mut truncated = 0;
        loop {
            match running.updates.recv() {
                Ok(Update::Chunk(found)) => {
                    matches.extend(found.matches);
                    truncated += found.truncated;
                }
                Ok(Update::Finished(outcome)) => {
                    matches.sort_by_key(|m| (m.line, m.start));
                    return (matches, truncated, outcome);
                }
                Err(_) => panic!("the worker died without reporting an outcome"),
            }
        }
    }

    /// The set-wide row space, which is the whole reason this module exists rather than callers
    /// using `search.rs` directly.
    ///
    /// Two members of four lines each: a match on the *second* member's line 1 must come back as row
    /// 5, not row 1. Getting this wrong is not a visible failure — it is a search that scrolls to a
    /// plausible wrong line in the previous file.
    #[test]
    fn a_match_in_an_older_member_carries_its_set_wide_row() {
        let found = shift(
            &Found {
                matches: vec![Match {
                    line: 1,
                    start: 0,
                    end: 3,
                }],
                truncated: 0,
                scanned: 4,
            },
            4,
        );
        assert_eq!(found.matches[0].line, 5);
        assert_eq!(found.scanned, 4, "shifting rows must not touch the counts");
    }

    #[test]
    fn an_empty_set_finishes_rather_than_hanging() {
        let running = start("x", false, Vec::new(), SearchOptions::default()).expect("start");
        let (matches, _, outcome) = collect(&running);
        assert!(matches.is_empty());
        assert_eq!(outcome, Outcome::Complete);
    }

    #[cfg(windows)]
    fn scratch(name: &str, body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("tailhawk-find-{name}.log"));
        std::fs::write(&path, body).expect("write fixture");
        path
    }

    #[cfg(windows)]
    fn open(path: &std::path::Path, first_row: u64) -> Excerpt {
        let file = LogFile::open(path).expect("open");
        let end = file.len().expect("len");
        let index = build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default())
            .expect("build index");
        Excerpt {
            file: Arc::new(file),
            charset: Charset::UTF_8,
            index,
            first_row,
        }
    }

    /// **A typo is answered before a thread exists**, so the find bar can say so on the keystroke
    /// rather than a timer tick later.
    ///
    /// It takes a real member because that is where the compile happens — one per member, per
    /// §7.4's charset-dependent engine choice. A set with no members compiles nothing and so
    /// validates nothing, which is a shape no document produces: `LogSet` is never empty.
    #[cfg(windows)]
    #[test]
    fn a_pattern_the_engine_refuses_fails_before_a_worker_starts() {
        let path = scratch("badpattern", "anything\n");
        let err = start(
            "(unclosed",
            false,
            vec![open(&path, 0)],
            SearchOptions::default(),
        )
        .err()
        .expect("an unclosed group is not a pattern");
        assert!(
            err.to_string().contains('('),
            "the error should name what was wrong with the pattern, got {err}"
        );
        let _ = std::fs::remove_file(path);
    }

    /// End to end on real files: two members, one search, set-wide rows.
    #[cfg(windows)]
    #[test]
    fn a_search_over_two_members_reports_one_row_space() {
        let older = scratch("older", "alpha\nbeta ERROR one\ngamma\ndelta\n");
        let newer = scratch("newer", "epsilon\nzeta ERROR two\neta\n");
        let members = vec![open(&older, 0), open(&newer, 4)];

        let running = start("ERROR", false, members, SearchOptions::default()).expect("start");
        let (matches, truncated, outcome) = collect(&running);

        assert_eq!(outcome, Outcome::Complete);
        assert_eq!(truncated, 0);
        let rows: Vec<u64> = matches.iter().map(|m| m.line).collect();
        assert_eq!(
            rows,
            vec![1, 5],
            "the second member's line 1 is the set's row 5"
        );
        let _ = std::fs::remove_file(older);
        let _ = std::fs::remove_file(newer);
    }

    /// **A cancelled pass says so**, and does not report the count it had reached as final.
    #[cfg(windows)]
    #[test]
    fn a_cancelled_search_reports_cancelled_rather_than_complete() {
        let path = scratch("cancelled", &"needle here\n".repeat(20_000));
        let members = vec![open(&path, 0)];

        // One thread and small chunks, so there is a pass still to cancel after the first chunk.
        let running = start(
            "needle",
            false,
            members,
            SearchOptions {
                lines_per_chunk: 500,
                threads: 1,
                ..SearchOptions::default()
            },
        )
        .expect("start");
        running.cancel();

        let (_, _, outcome) = collect(&running);
        assert_eq!(outcome, Outcome::Cancelled);
        let _ = std::fs::remove_file(path);
    }

    /// The cap is reported rather than looking like the end of the matches.
    ///
    /// **One member, deliberately.** With two, the check at the top of the next member catches it,
    /// and a set of one — every ordinary log — would have fallen through the loop reporting
    /// `Complete`.
    #[cfg(windows)]
    #[test]
    fn hitting_the_match_cap_is_not_reported_as_completion() {
        let path = scratch("capped", &"needle\n".repeat(5_000));
        let members = vec![open(&path, 0)];

        let running = start(
            "needle",
            false,
            members,
            SearchOptions {
                max_matches: 100,
                lines_per_chunk: 100,
                threads: 1,
            },
        )
        .expect("start");
        let (matches, _, outcome) = collect(&running);

        assert_eq!(outcome, Outcome::Capped);
        assert!(
            matches.len() >= 100,
            "the cap stops the pass, it does not discard what was found: {}",
            matches.len()
        );
        let _ = std::fs::remove_file(path);
    }
}
