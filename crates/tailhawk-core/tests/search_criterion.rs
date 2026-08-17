//! `PLAN.md`'s M5 search criterion, run against a real fixture.
//!
//! > "full-file regex search streams first match on a 10 GB fixture; a pathological lookaround
//! > degrades to *pattern too slow, truncated* rather than hanging"
//!
//! **Ignored by default and pointed at a fixture by environment variable**, because a 10 GB file is
//! not something `cargo test` should build or require. The generator that built the current fixture
//! lived in an untracked `scratchpad/` and **did not survive a reboot**, which is why `tools/` is
//! now where a harness goes; the fixture it produced is described below and in `HANDOFF.md` in
//! enough detail to rebuild. Then:
//!
//! ```text
//! TAILHAWK_BIG_LOG=C:\tmp\th-10gb.log cargo test -p tailhawk-core --test search_criterion -- --ignored --nocapture
//! ```
//!
//! The fixture carries three planted lines: `CANARY_ALPHA` at 1% of the file, `CANARY_OMEGA` at 99%,
//! and a 60 KB line containing `CANARY_BURIED` at 50%. The first two are what make "streams the
//! first match" a testable claim rather than a hopeful one — a search that only reported at the end
//! would report both at the same moment.

use std::sync::Arc;
use std::time::Instant;

use tailhawk_core::encoding::Charset;
use tailhawk_core::file::LogFile;
use tailhawk_core::find::{self, Excerpt, Outcome, Update};
use tailhawk_core::indexer::{build_index, IndexOptions};
use tailhawk_core::search::{Pattern, Search, SearchOptions, FANCY_LINE_CAP};

fn fixture() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("TAILHAWK_BIG_LOG")?;
    let path = std::path::PathBuf::from(path);
    path.exists().then_some(path)
}

/// §7.4: "Results stream — first match visible quickly, total count arriving later."
///
/// The assertion is a **ratio, not a duration**: the early canary must be reported in a small
/// fraction of the time the whole pass takes. A wall-clock threshold would flake on a loaded machine
/// and would have to be retuned for every disk it ever runs on.
#[test]
#[ignore = "needs TAILHAWK_BIG_LOG pointing at a large fixture"]
fn a_full_file_search_streams_its_first_match_long_before_it_finishes() {
    let Some(path) = fixture() else {
        eprintln!("skipped: set TAILHAWK_BIG_LOG");
        return;
    };
    let file = LogFile::open(&path).expect("open");
    let end = file.len().expect("len");

    let began = Instant::now();
    let index =
        build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("index");
    let indexed = began.elapsed();
    println!(
        "INDEX  {:.2} GB, {} lines, {:.2}s ({:.0} MB/s)",
        end as f64 / 1e9,
        index.line_count(),
        indexed.as_secs_f64(),
        end as f64 / 1e6 / indexed.as_secs_f64()
    );

    let pattern = Pattern::compile("CANARY_(ALPHA|OMEGA)", Charset::UTF_8, false).expect("compile");
    let search = Search::new(pattern, Charset::UTF_8, SearchOptions::default());

    let began = Instant::now();
    let mut first_match_at = None;
    let found = search
        .run(&file, &index, |chunk| {
            if first_match_at.is_none() && !chunk.matches.is_empty() {
                first_match_at = Some(began.elapsed());
            }
        })
        .expect("search");
    let whole = began.elapsed();

    let first = first_match_at.expect("something must have matched");
    println!(
        "SEARCH first match {:.3}s, whole pass {:.3}s ({:.0} MB/s), {} matches, {} truncated",
        first.as_secs_f64(),
        whole.as_secs_f64(),
        end as f64 / 1e6 / whole.as_secs_f64(),
        found.matches.len(),
        found.truncated
    );

    assert_eq!(found.matches.len(), 2, "both canaries, and nothing else");
    assert!(
        first.as_secs_f64() < whole.as_secs_f64() * 0.5,
        "the first match arrived at {:.3}s of a {:.3}s pass, which is not streaming",
        first.as_secs_f64(),
        whole.as_secs_f64()
    );
}

/// The same criterion **through the worker the UI actually uses**, which is a different claim.
///
/// The test above proves the engine streams. It calls [`Search::run`] directly, on the calling
/// thread, against an index nothing else is touching — none of which is how the window searches. The
/// shell hands [`find::start`] a snapshot and drains a channel from a timer, and everything that can
/// go wrong in *that* — the handles, the cloned index, the row mapping, the outcome — is between the
/// engine and the screen.
///
/// So this measures the same two numbers one layer up, and adds the one the other test cannot make:
/// that the matches arrive **while the pass is still running**, on a channel, from another thread.
#[test]
#[ignore = "needs TAILHAWK_BIG_LOG pointing at a large fixture"]
fn the_find_worker_streams_matches_from_a_snapshot_of_ten_gigabytes() {
    let Some(path) = fixture() else {
        eprintln!("skipped: set TAILHAWK_BIG_LOG");
        return;
    };
    let file = LogFile::open(&path).expect("open");
    let end = file.len().expect("len");
    let index =
        build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("index");
    let lines = index.line_count();

    let excerpt = Excerpt {
        file: Arc::new(file),
        charset: Charset::UTF_8,
        index,
        first_row: 0,
    };

    let began = Instant::now();
    let running = find::start(
        "CANARY_(ALPHA|OMEGA)",
        false,
        vec![excerpt],
        SearchOptions::default(),
    )
    .expect("start");

    let (mut matches, mut first_match_at, mut outcome) = (Vec::new(), None, None);
    while outcome.is_none() {
        for update in running.drain() {
            match update {
                Update::Chunk(found) => {
                    if first_match_at.is_none() && !found.matches.is_empty() {
                        first_match_at = Some(began.elapsed());
                    }
                    matches.extend(found.matches);
                }
                Update::Finished(done) => outcome = Some(done),
            }
        }
        // The shell drains from a 100 ms timer; this is that, without the timer.
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let whole = began.elapsed();
    let first = first_match_at.expect("something must have matched");

    println!(
        "WORKER first match {:.3}s, whole pass {:.3}s ({:.0} MB/s), {} matches over {} lines, {:?}",
        first.as_secs_f64(),
        whole.as_secs_f64(),
        end as f64 / 1e6 / whole.as_secs_f64(),
        matches.len(),
        lines,
        outcome.as_ref().expect("an outcome")
    );

    assert_eq!(outcome, Some(Outcome::Complete));
    assert_eq!(matches.len(), 2, "both canaries, and nothing else");
    assert!(
        first.as_secs_f64() < whole.as_secs_f64() * 0.5,
        "the first match reached the channel at {:.3}s of a {:.3}s pass, which is not streaming",
        first.as_secs_f64(),
        whole.as_secs_f64()
    );
}

/// §7.4's other half: "a pathological lookaround degrades to *pattern too slow, truncated* rather
/// than hanging."
///
/// The pattern is nested quantifiers behind a lookbehind — the classic catastrophic-backtracking
/// shape — pointed at a fixture containing a 60 KB line. Without §7.4's caps this does not return.
#[test]
#[ignore = "needs TAILHAWK_BIG_LOG pointing at a large fixture"]
fn a_pathological_lookaround_finishes_and_says_it_truncated() {
    let Some(path) = fixture() else {
        eprintln!("skipped: set TAILHAWK_BIG_LOG");
        return;
    };
    let file = LogFile::open(&path).expect("open");
    let end = file.len().expect("len");
    let index =
        build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("index");

    let source = r"(?<=WARN )(x+x+)+y";
    let pattern = Pattern::compile(source, Charset::UTF_8, false).expect("compile");
    assert!(
        pattern.backtracking(),
        "this pattern must reach the backtracking engine or the test proves nothing"
    );

    let began = Instant::now();
    let found = Search::new(pattern, Charset::UTF_8, SearchOptions::default())
        .run(&file, &index, |_| {})
        .expect("search");
    let took = began.elapsed();

    println!(
        "FANCY  {:.3}s over {} lines, {} truncated, cap {} bytes",
        took.as_secs_f64(),
        found.scanned,
        found.truncated,
        FANCY_LINE_CAP
    );

    // Finishing at all is the assertion. Without the per-line cap and the backtrack limit this
    // pattern does not return on a 60 KB line, which is the hang class §7.4 names by issue number.
    assert!(found.scanned > 0);
    assert!(
        found.truncated > 0,
        "the 60 KB line must be reported as truncated rather than silently skipped"
    );
}

/// §7.3, measured: "every filter change is a full-file pass" that streams. Two include chips and
/// one exclude over the same 10 GB — `CANARY` kept, `BURIED` dropped — so the survivors are the two
/// canaries and the pass has to decode and evaluate every line rather than scan bytes for a
/// literal, which is what makes its speed a different number from the search's.
#[test]
#[ignore = "needs TAILHAWK_BIG_LOG pointing at a large fixture"]
fn the_filter_worker_streams_survivors_from_a_snapshot_of_ten_gigabytes() {
    use tailhawk_core::filter::{Chip, Chips, Polarity};
    use tailhawk_core::sieve;

    let Some(path) = fixture() else {
        eprintln!("skipped: set TAILHAWK_BIG_LOG");
        return;
    };
    let file = LogFile::open(&path).expect("open");
    let end = file.len().expect("len");
    let index =
        build_index(&file, Charset::UTF_8, 0, end, &IndexOptions::default()).expect("index");
    let lines = index.line_count();
    let excerpt = Excerpt {
        file: Arc::new(file),
        charset: Charset::UTF_8,
        index,
        first_row: 0,
    };

    let chips = Chips {
        chips: vec![
            Chip::parse("canary", Polarity::Include).expect("chip"),
            Chip::parse("/CANARY_[A-Z]+/", Polarity::Include).expect("chip"),
            Chip::parse("buried", Polarity::Exclude).expect("chip"),
        ],
    };

    let began = Instant::now();
    let running =
        sieve::start(chips, vec![excerpt], 0, lines, SearchOptions::default()).expect("start");

    let (mut rows, mut first_at, mut outcome, mut scanned) = (Vec::new(), None, None, 0u64);
    while outcome.is_none() {
        for update in running.drain() {
            match update {
                sieve::Update::Chunk(kept) => {
                    if first_at.is_none() && !kept.rows.is_empty() {
                        first_at = Some(began.elapsed());
                    }
                    scanned += kept.scanned;
                    rows.extend(kept.rows);
                }
                sieve::Update::Finished(done) => outcome = Some(done),
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let whole = began.elapsed();
    let first = first_at.expect("something must have survived");
    rows.sort_unstable();

    println!(
        "FILTER first survivor {:.3}s, whole pass {:.3}s ({:.0} MB/s), {} survivors, {} of {} lines scanned, {:?}",
        first.as_secs_f64(),
        whole.as_secs_f64(),
        end as f64 / 1e6 / whole.as_secs_f64(),
        rows.len(),
        scanned,
        lines,
        outcome.as_ref().expect("an outcome")
    );

    assert_eq!(outcome, Some(Outcome::Complete));
    assert_eq!(scanned, lines, "every line is examined");
    assert_eq!(
        rows.len(),
        2,
        "both canaries survive and the buried one is excluded"
    );
    assert!(
        first.as_secs_f64() < whole.as_secs_f64() * 0.5,
        "the first survivor reached the channel at {:.3}s of a {:.3}s pass, which is not streaming",
        first.as_secs_f64(),
        whole.as_secs_f64()
    );
}
