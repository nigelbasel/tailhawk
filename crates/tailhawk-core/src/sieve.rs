//! The filter pass, on a worker, over a snapshot of the log — `SPEC.md` §7.3.
//!
//! §7.3 is honest about the cost: "a hide-non-matching view cannot render row *N* without knowing
//! which records survive up to *N* … **every filter change is a full-file pass.**" So the pass is
//! the same shape as [`crate::find`]'s search — an [`Excerpt`] per member, one worker, results
//! streamed on a channel, cancellable — and it reuses `search.rs`'s chunked line walk rather than
//! having its own. What it streams is different: not matches with byte ranges, but **the set-wide
//! row numbers that survive the chips**, in the order the chunks finish.
//!
//! ## Row numbers, not a bitmap — and why
//!
//! The consumer is a derived row space: view row *k* is the *k*-th surviving row, and the scrollbar
//! is sized by how many there are. A sorted `Vec<u64>` answers both directly and is maintained the
//! way `find.rs`'s match list is — chunks are disjoint runs of lines, so each lands in one
//! contiguous slot by `partition_point` and splice. A bitmap is smaller (one bit a row against
//! eight bytes a survivor) but cannot answer "the *k*-th survivor" without a rank structure on top,
//! and §7.3 sizes nothing that says the vector is too big; a filter that keeps a tenth of a
//! 100-million-line file is 80 MB, and a filter that keeps everything is the unfiltered view and
//! should not be one. **If a measurement disagrees, a bitmap with rank is the fallback**, behind
//! the same `Update`.
//!
//! ## Rows appended after the snapshot are sieved by the same pass over a range
//!
//! A tail with a filter on it is *the* use of a filter, and the lines a writer appends after the
//! snapshot was taken have to be sieved too. They are not evaluated on the window thread as they
//! arrive: at §11.3's 50 MB/s that is half a million lines a second through a regex on the thread
//! that paints. [`start`] takes a **row range**, so the same worker shape sieves the new rows —
//! `[old_total, new_total)` — and streams them into the same list.
//!
//! ## What is deliberately not here
//!
//! - **The debounce**, as in `find.rs`: the cancellation half is here, the 300 ms timer is UI.
//! - **The derived row space itself** — the mapping from view rows to file rows, the scattered
//!   fetch it needs from `rows.rs`, the two view modes. This is the pass; that is the shell.
//! - **Structure.** Every line is evaluated as [`Record::unparsed`], so a chip on `level` or a
//!   column evaluates to *unknown* (§7.2) until M6's format detection produces records. A `bare_text`
//!   or `/regex/` chip — the owner's include, exclude and composing text filters — works now.

use std::sync::mpsc::{Receiver, Sender};

use crate::filter::Chips;
use crate::find::{Excerpt, Outcome};
use crate::format::Format;
use crate::indexer::ChunkReader;
use crate::record::Record;
use crate::search::{each_line, run_chunked, Cancel, SearchOptions};
use crate::Result;

/// One chunk's survivors, **in set-wide rows**, ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Kept {
    pub rows: Vec<u64>,
    /// Lines examined for this chunk, survivors or not — what a progress counter climbs by.
    pub scanned: u64,
}

/// What a running pass reports. Chunks arrive out of order; see [`run_chunked`].
#[derive(Debug)]
pub enum Update {
    Chunk(Kept),
    /// The last message on the channel.
    Finished(Outcome),
}

/// A pass in flight. Dropping it stops the worker and joins it, as [`crate::find::Running`] does.
pub struct Running {
    cancel: Cancel,
    updates: Receiver<Update>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    /// Everything reported since the last call. **Never blocks.**
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
            let _ = worker.join();
        }
    }
}

/// Starts a pass over the set-wide rows `[from, to)` of `members`, streaming survivors.
///
/// `from..to` is what lets the same worker sieve a snapshot (`0..total`) and, later, only what a
/// writer has appended since. Members whose rows fall entirely outside the range are skipped
/// without a read; the rest are clipped to it. `chips` is moved to the worker: it is `Clone`, and
/// a caller keeps its own copy for the title and the next edit.
/// `records_only` keeps only lines the format calls first lines — §6.4's "continuations are
/// collapsed" — composed with the chips: a row survives if it is a first line *and* the chips keep
/// it. Without a format it means nothing and keeps everything.
pub fn start(
    chips: Chips,
    format: Option<&'static Format>,
    records_only: bool,
    members: Vec<Excerpt>,
    from: u64,
    to: u64,
    options: SearchOptions,
) -> Result<Running> {
    let cancel = Cancel::new();
    let (tx, updates) = std::sync::mpsc::channel();
    let worker = {
        let cancel = cancel.clone();
        std::thread::Builder::new()
            .name("tailhawk-filter".to_owned())
            .spawn(move || {
                run(
                    Job {
                        chips,
                        format,
                        records_only,
                    },
                    members,
                    from..to,
                    options,
                    &cancel,
                    &tx,
                )
            })
            .map_err(|e| crate::Error(format!("starting the filter worker: {e}")))?
    };
    Ok(Running {
        cancel,
        updates,
        worker: Some(worker),
    })
}

/// The worker body: every member in row order, clipped to `[from, to)`, under one cancel flag.
/// What one pass evaluates every line against.
struct Job {
    chips: Chips,
    format: Option<&'static Format>,
    records_only: bool,
}

fn run(
    job: Job,
    members: Vec<Excerpt>,
    rows: std::ops::Range<u64>,
    options: SearchOptions,
    cancel: &Cancel,
    tx: &Sender<Update>,
) {
    let mut outcome = Outcome::Complete;
    for member in members {
        if cancel.cancelled() {
            outcome = Outcome::Cancelled;
            break;
        }
        // The member's rows in set space are `first_row..first_row + lines`; clip to the range.
        let lines = member.index.line_count();
        let begin = rows.start.max(member.first_row);
        let end = rows.end.min(member.first_row.saturating_add(lines));
        if begin >= end {
            continue;
        }
        let sieve = Sieve {
            chips: &job.chips,
            format: job.format,
            records_only: job.records_only,
            charset: member.charset,
            options,
            cancel: cancel.clone(),
        };
        let sent = sieve.run(
            &*member.file,
            &member.index,
            begin - member.first_row,
            end - member.first_row,
            |kept| {
                let _ = tx.send(Update::Chunk(shift(kept, member.first_row)));
            },
        );
        if let Err(e) = sent {
            outcome = Outcome::Failed(e.to_string());
            break;
        }
    }
    // Asked last, and it wins over completion, for the reason `find.rs` gives: a pass stopped
    // between its last chunk and here has not examined the rest.
    if cancel.cancelled() {
        outcome = Outcome::Cancelled;
    }
    let _ = tx.send(Update::Finished(outcome));
}

fn shift(kept: &Kept, first_row: u64) -> Kept {
    Kept {
        rows: kept.rows.iter().map(|r| r + first_row).collect(),
        scanned: kept.scanned,
    }
}

/// One member's pass: the chips against every line of `[from, to)`, member-local rows.
struct Sieve<'a> {
    chips: &'a Chips,
    /// The detected format, when a chip names a field: `level >= Warning` needs the record, and
    /// only a parse produces one. Text-only chips never pay for it.
    format: Option<&'static Format>,
    /// §6.4's collapse: drop lines that are not first lines under `format`.
    records_only: bool,
    charset: crate::encoding::Charset,
    options: SearchOptions,
    cancel: Cancel,
}

impl Sieve<'_> {
    /// Runs the pass, reporting each chunk's survivors as it completes; returns them all, sorted.
    fn run<R: ChunkReader + ?Sized>(
        &self,
        reader: &R,
        index: &crate::index::LineIndex,
        from: u64,
        to: u64,
        on_chunk: impl FnMut(&Kept) + Send,
    ) -> Result<Kept> {
        // `run_chunked` chunks `[0, total)`; offsetting by `from` keeps one chunking rule.
        let chunks = run_chunked(
            to.saturating_sub(from),
            self.options.lines_per_chunk,
            self.options.threads,
            self.cancel.flag(),
            || false,
            |a, b| self.sieve_lines(reader, index, from + a, from + b),
            on_chunk,
        )?;
        let mut all = Kept::default();
        for chunk in chunks {
            all.rows.extend(chunk.rows);
            all.scanned += chunk.scanned;
        }
        all.rows.sort_unstable();
        Ok(all)
    }

    /// Lines `[from, to)` through the chips. **One `Record`, refilled**, because a `String` per
    /// line is a hundred million allocations over the file §7.3 says this pass reads.
    fn sieve_lines<R: ChunkReader + ?Sized>(
        &self,
        reader: &R,
        index: &crate::index::LineIndex,
        from: u64,
        to: u64,
    ) -> Result<Kept> {
        let mut kept = Kept::default();
        let mut record = Record::unparsed(String::new());
        let parse = self.format.filter(|_| {
            self.chips
                .chips
                .iter()
                .any(|c| !c.predicate.fields().is_empty())
        });
        each_line(
            reader,
            self.charset,
            index,
            from,
            to,
            self.cancel.flag(),
            |line, text| {
                if self.records_only && self.format.is_some_and(|f| !f.is_first_line(text)) {
                    kept.scanned += 1;
                    return;
                }
                let keeps = match parse.and_then(|f| f.parse(text)) {
                    Some(parsed) => self.chips.keeps(&parsed),
                    None => {
                        record.raw.clear();
                        record.raw.push_str(text);
                        self.chips.keeps(&record)
                    }
                };
                if keeps {
                    kept.rows.push(line);
                }
                kept.scanned += 1;
            },
        )?;
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Charset;
    use crate::filter::{Chip, Polarity};
    use crate::indexer::{build_index, IndexOptions};
    use std::sync::Arc;

    const UTF8: Charset = Charset::UTF_8;

    fn chips(include: &[&str], exclude: &[&str]) -> Chips {
        let mut c = Chips::default();
        for text in include {
            c.chips
                .push(Chip::parse(text, Polarity::Include).expect("include"));
        }
        for text in exclude {
            c.chips
                .push(Chip::parse(text, Polarity::Exclude).expect("exclude"));
        }
        c
    }

    fn excerpt(path: &std::path::Path, text: &str, first_row: u64) -> Excerpt {
        std::fs::write(path, text).expect("write the fixture");
        let file = Arc::new(crate::file::LogFile::open(path).expect("open"));
        let index = build_index(&*file, UTF8, 0, text.len() as u64, &IndexOptions::default())
            .expect("index");
        Excerpt {
            file,
            charset: UTF8,
            index,
            first_row,
        }
    }

    /// Collects a whole run, in the order a caller would, then sorts as the shell does.
    fn collect(running: &Running) -> (Vec<u64>, u64, Outcome) {
        let mut rows = Vec::new();
        let mut scanned = 0;
        loop {
            match running.updates.recv() {
                Ok(Update::Chunk(kept)) => {
                    rows.extend(kept.rows);
                    scanned += kept.scanned;
                }
                Ok(Update::Finished(outcome)) => {
                    rows.sort_unstable();
                    return (rows, scanned, outcome);
                }
                Err(_) => panic!("the worker hung up without finishing"),
            }
        }
    }

    fn fixture(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    /// The owner's daily case: an include, an exclude, and both composing. §7.3's model.
    #[test]
    fn include_then_exclude_over_a_real_file() {
        let path = fixture("tailhawk_sieve_basic.log");
        let text = "INFO start\nERROR boom\nDEBUG noise\nERROR again (retrying)\nWARN retrying\n";
        let members = vec![excerpt(&path, text, 0)];
        let running = start(
            chips(&["error"], &["retrying"]),
            None,
            false,
            members,
            0,
            5,
            SearchOptions::default(),
        )
        .expect("start");
        let (rows, scanned, outcome) = collect(&running);
        assert_eq!(outcome, Outcome::Complete);
        assert_eq!(scanned, 5);
        assert_eq!(
            rows,
            [1],
            "ERROR boom survives; ERROR again is excluded by 'retrying'"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// No chips is the unfiltered view: every row survives.
    #[test]
    fn no_chips_keeps_every_row() {
        let path = fixture("tailhawk_sieve_none.log");
        let members = vec![excerpt(&path, "a\nb\nc\n", 0)];
        let running = start(
            Chips::default(),
            None,
            false,
            members,
            0,
            3,
            SearchOptions::default(),
        )
        .expect("start");
        let (rows, _, outcome) = collect(&running);
        assert_eq!((rows, outcome), (vec![0, 1, 2], Outcome::Complete));
        let _ = std::fs::remove_file(&path);
    }

    /// Two members, one row space: survivors come back in set-wide rows, and a member outside the
    /// requested range is not read at all.
    #[test]
    fn a_set_reports_set_wide_rows_and_a_range_clips_it() {
        let a = fixture("tailhawk_sieve_a.log");
        let b = fixture("tailhawk_sieve_b.log");
        let members = vec![
            excerpt(&a, "x1\ny\nx2\n", 0),
            excerpt(&b, "x3\ny\ny\nx4\n", 3),
        ];
        let running = start(
            chips(&["x"], &[]),
            None,
            false,
            members,
            0,
            7,
            SearchOptions::default(),
        )
        .expect("start");
        let (rows, scanned, _) = collect(&running);
        assert_eq!(rows, [0, 2, 3, 6]);
        assert_eq!(scanned, 7);

        // Only the rows appended "after the snapshot": the second member's last two.
        let members = vec![
            excerpt(&a, "x1\ny\nx2\n", 0),
            excerpt(&b, "x3\ny\ny\nx4\n", 3),
        ];
        let running = start(
            chips(&["x"], &[]),
            None,
            false,
            members,
            5,
            7,
            SearchOptions::default(),
        )
        .expect("start");
        let (rows, scanned, _) = collect(&running);
        assert_eq!(rows, [6]);
        assert_eq!(
            scanned, 2,
            "the first member and the clipped rows are not read"
        );
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }

    /// Chunks arrive out of order and each is internally ascending — the contract the shell's
    /// splice relies on — and a small `lines_per_chunk` makes that observable.
    #[test]
    fn each_chunk_is_ascending_and_covers_a_disjoint_run() {
        let path = fixture("tailhawk_sieve_chunks.log");
        let text: String = (0..1000).map(|i| format!("row {i}\n")).collect();
        let members = vec![excerpt(&path, &text, 0)];
        let running = start(
            chips(&["row"], &[]),
            None,
            false,
            members,
            0,
            1000,
            SearchOptions {
                lines_per_chunk: 64,
                threads: 4,
                ..SearchOptions::default()
            },
        )
        .expect("start");
        let mut chunks = Vec::new();
        loop {
            match running.updates.recv() {
                Ok(Update::Chunk(kept)) => {
                    assert!(kept.rows.windows(2).all(|w| w[0] < w[1]), "{:?}", kept.rows);
                    chunks.push(kept.rows);
                }
                Ok(Update::Finished(outcome)) => {
                    assert_eq!(outcome, Outcome::Complete);
                    break;
                }
                Err(_) => panic!("hung up"),
            }
        }
        assert!(chunks.len() > 1, "small chunks should mean several reports");
        let mut all: Vec<u64> = chunks.concat();
        all.sort_unstable();
        assert_eq!(all, (0..1000).collect::<Vec<_>>());
        let _ = std::fs::remove_file(&path);
    }

    /// Dropping the handle stops the worker and says so.
    #[test]
    fn cancelling_is_reported_as_cancelled() {
        let path = fixture("tailhawk_sieve_cancel.log");
        let text: String = (0..200_000)
            .map(|i| format!("row {i} some padding text\n"))
            .collect();
        let members = vec![excerpt(&path, &text, 0)];
        let running = start(
            chips(&["row"], &[]),
            None,
            false,
            members,
            0,
            200_000,
            SearchOptions {
                lines_per_chunk: 1000,
                threads: 1,
                ..SearchOptions::default()
            },
        )
        .expect("start");
        running.cancel();
        let (_, scanned, outcome) = collect(&running);
        assert_eq!(outcome, Outcome::Cancelled);
        assert!(scanned < 200_000, "a cancelled pass did not read it all");
        let _ = std::fs::remove_file(&path);
    }
    /// A chip that names a field — `level >= Warning` — evaluates against the parsed record when a
    /// format is given, and to *unknown* (which an include chip drops) when it is not. §7.2's rule,
    /// end to end through the pass.
    #[test]
    fn a_field_chip_needs_the_format_and_gets_it() {
        let path = fixture("tailhawk_sieve_fields.log");
        let text = "2026-08-16 09:14:02.117 +02:00 [INF] started\n\
                    2026-08-16 09:14:03.884 +02:00 [ERR] failed\n\
                    2026-08-16 09:14:04.002 +02:00 [WRN] retrying\n";
        let serilog = crate::format::by_id("serilog-file").expect("catalogue");

        let members = vec![excerpt(&path, text, 0)];
        let running = start(
            chips(&["level >= Warning"], &[]),
            Some(serilog),
            false,
            members,
            0,
            3,
            SearchOptions::default(),
        )
        .expect("start");
        let (rows, _, outcome) = collect(&running);
        assert_eq!((rows, outcome), (vec![1, 2], Outcome::Complete));

        let members = vec![excerpt(&path, text, 0)];
        let running = start(
            chips(&["level >= Warning"], &[]),
            None,
            false,
            members,
            0,
            3,
            SearchOptions::default(),
        )
        .expect("start");
        let (rows, _, _) = collect(&running);
        assert!(
            rows.is_empty(),
            "with no format the field is unknown and an include drops all"
        );
        let _ = std::fs::remove_file(&path);
    }
}
