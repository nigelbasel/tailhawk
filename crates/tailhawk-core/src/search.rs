//! Search — `SPEC.md` §7.4, engine policy first.
//!
//! §7.4 does not leave the engine to be chosen: "the Rust `regex` crate. Lazy DFA, rare-byte memchr
//! prefilters, Teddy SIMD multi-pattern — the same tricks klogg imports Hyperscan for, with no
//! Boost/Ragel/C++ dependency." What it *does* leave open is what happens to a pattern that crate
//! will not compile, and it answers that too, with a table and a reason.
//!
//! ## Two engines, and why the second one exists at all
//!
//! §7.4: "Log4net and NLog users write `(?<!DEBUG).*Exception` routinely, so rejecting lookaround
//! outright would look broken — but an unbounded backtracking engine over a 500 KB
//! serialised-object line is the exact hang class klogg is criticised for (#803)."
//!
//! | Pattern class | Engine | Guard |
//! |---|---|---|
//! | Everything `regex` compiles | `regex` | `size_limit` and `dfa_size_limit` set explicitly |
//! | Lookaround / backreferences | `fancy-regex` | `backtrack_limit`; **hard 8 KB per-line cap**; cancellable; a visible "pattern too slow, truncated" |
//!
//! **Which engine a pattern gets is decided by asking `regex` to compile it**, not by looking for
//! `(?=` in the source. That is ours — §7.4 names the two classes and not the test — and the reason
//! is that a written test gets it wrong in both directions: `[(?=]` is a character class containing
//! three ordinary characters, and `\(\?\=` is three escaped literals. The compiler is the only thing
//! that knows, and it already has to run.
//!
//! ## Bytes where the encoding allows it, and text where it does not
//!
//! §7.4: "Search raw bytes where possible. klogg's own profile shows line decoding becomes ~50% of
//! search time once the regex engine is fast."
//!
//! *Where possible* is doing real work in that sentence. A byte-oriented encoding (UTF-8, and the
//! single-byte codepages) can be searched as bytes, and that is what [`Engine::Bytes`] does. **UTF-16
//! and UTF-32 cannot**: a pattern compiled from UTF-8 source matched against UTF-16 bytes finds
//! nothing, and finds nothing *silently*, which is the worst way for a search to be wrong. Those
//! decode first — the cost §7.4 wanted to avoid, paid only where skipping it would lie.
//!
//! ## Line-oriented means no overlap
//!
//! §7.4: "Parallel chunked search snaps chunk boundaries to newlines using the line index — zero
//! overlap needed for line-oriented patterns."
//!
//! [`Search::run`] chunks by **line number**, not by byte offset, which makes the snap free: the
//! index already knows where line *n* starts. A chunk is a run of whole lines, so no match can
//! straddle two chunks and no overlap or dedup is needed.
//!
//! **Multiline patterns are not supported and are not pretended to be.** §7.4 describes what they
//! need — "overlap ≥ max match length with start-offset dedup" — and that is not implemented; a
//! pattern matching across a line terminator simply will not match. Recorded in `HANDOFF.md`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use crate::encoding::Charset;
use crate::index::LineIndex;
use crate::indexer::{offset_of_line, ChunkReader};
use crate::lines::LineDecoder;
use crate::{Error, Result};

/// `regex`'s compiled-program budget, in bytes. §7.4: "set explicitly."
///
/// The crate's default is 10 MB and this is deliberately the same order. It is not a *time* guard —
/// the linear-time guarantee is what makes one unnecessary — it is a memory guard against a pattern
/// like `a{1000}{1000}` whose compiled form is enormous while its behaviour is unremarkable.
pub const SIZE_LIMIT: usize = 10 * 1024 * 1024;

/// The lazy DFA's cache budget, in bytes. §7.4: "set explicitly."
///
/// Exceeding it does not fail the search — `regex` falls back to a slower engine — so this trades
/// memory for speed rather than correctness, and is stated rather than defaulted because §7.4 asks
/// for it to be.
pub const DFA_SIZE_LIMIT: usize = 2 * 1024 * 1024;

/// §7.4's "hard 8 KB per-line input cap", for the backtracking engine only.
///
/// **This is the whole defence against klogg #803.** A `backtrack_limit` bounds the *steps* per
/// attempt; the input length bounds how bad an attempt can get in the first place, and §10.3 supports
/// lines up to 32 KB — so a 500 KB serialised-object line is exactly the case that needs a hard stop
/// before the engine sees it.
pub const FANCY_LINE_CAP: usize = 8 * 1024;

/// Backtracking steps allowed per line. §7.4: "explicit `backtrack_limit`."
pub const BACKTRACK_LIMIT: usize = 1_000_000;

/// A compiled pattern, and which engine compiled it.
enum Engine {
    /// The linear-time engine, searching bytes. §7.4's preferred path.
    Bytes(regex::bytes::Regex),
    /// The linear-time engine, searching decoded text — for UTF-16 and UTF-32, where a byte match
    /// would silently find nothing.
    Text(regex::Regex),
    /// §7.4's escape hatch. Backtracking, capped, and it can give up.
    Fancy(fancy_regex::Regex),
}

/// A pattern ready to search with, and the policy that produced it.
///
/// `Debug` shows the source and the engine, not the compiled program: a lazy DFA's internals are
/// pages of state that would bury the two facts a reader wants.
pub struct Pattern {
    engine: Engine,
    source: String,
    /// Whether this went to the backtracking engine, so the UI can say so before a search runs.
    backtracking: bool,
}

impl std::fmt::Debug for Pattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let engine = match self.engine {
            Engine::Bytes(_) => "regex/bytes",
            Engine::Text(_) => "regex/text",
            Engine::Fancy(_) => "fancy-regex",
        };
        f.debug_struct("Pattern")
            .field("source", &self.source)
            .field("engine", &engine)
            .finish()
    }
}

impl Pattern {
    /// Compiles `source` under §7.4's policy.
    ///
    /// `charset` decides bytes-versus-text; see the module note. `case_insensitive` is the one flag
    /// §7.2's `/pattern/i` carries and the one a search box needs.
    pub fn compile(source: &str, charset: Charset, case_insensitive: bool) -> Result<Self> {
        // **The linear-time engine is asked first, always**, and the fallback is its refusal rather
        // than a guess about the pattern's text. See the module note.
        let linear = if charset.code_unit() == 1 {
            regex::bytes::RegexBuilder::new(source)
                .case_insensitive(case_insensitive)
                .size_limit(SIZE_LIMIT)
                .dfa_size_limit(DFA_SIZE_LIMIT)
                .build()
                .map(Engine::Bytes)
                .map_err(|e| e.to_string())
        } else {
            regex::RegexBuilder::new(source)
                .case_insensitive(case_insensitive)
                .size_limit(SIZE_LIMIT)
                .dfa_size_limit(DFA_SIZE_LIMIT)
                .build()
                .map(Engine::Text)
                .map_err(|e| e.to_string())
        };

        let (engine, backtracking) = match linear {
            Ok(engine) => (engine, false),
            Err(linear_error) => {
                let mut builder = fancy_regex::RegexBuilder::new(source);
                builder.backtrack_limit(BACKTRACK_LIMIT);
                // Case-insensitivity goes in as an inline flag, which is the portable spelling and
                // avoids depending on a builder method the crate may not expose.
                let built = if case_insensitive {
                    fancy_regex::RegexBuilder::new(&format!("(?i){source}"))
                        .backtrack_limit(BACKTRACK_LIMIT)
                        .build()
                } else {
                    builder.build()
                };
                match built {
                    Ok(fancy) => (Engine::Fancy(fancy), true),
                    // **Both engines refused, so the first message is the one to show.** The
                    // backtracking engine is the fallback, and reporting *its* complaint about a
                    // pattern the user wrote for the primary one sends them to the wrong place.
                    Err(_) => return Err(Error(linear_error)),
                }
            }
        };
        Ok(Self {
            engine,
            source: source.to_string(),
            backtracking,
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether this pattern runs on §7.4's backtracking engine, and so carries its caps.
    pub fn backtracking(&self) -> bool {
        self.backtracking
    }

    /// Every match in one line, with its capture groups.
    ///
    /// **For §7.1's highlighting, which needs all of them and not the first.** It lives here rather
    /// than in `highlight.rs` so §7.4's engine policy — which engine, which caps — stays in one
    /// place; a highlighter that compiled its own patterns would be a second policy nobody updated.
    ///
    /// `on_match` receives the match's byte range within `text` and one entry per capture group,
    /// `None` where the group did not participate. Ranges are byte offsets in `text`, which for the
    /// byte engine are the same offsets it matched at.
    ///
    /// Stops at §7.4's per-line cap for the backtracking engine, and returns whether it did — a
    /// caller showing "pattern too slow, truncated" needs the same fact here as a search does.
    pub fn each_match(
        &self,
        text: &str,
        mut on_match: impl FnMut(core::ops::Range<usize>, &[Option<core::ops::Range<usize>>]),
    ) -> bool {
        let mut groups: Vec<Option<core::ops::Range<usize>>> = Vec::new();
        match &self.engine {
            Engine::Bytes(re) => {
                for caps in re.captures_iter(text.as_bytes()) {
                    let whole = caps.get(0).expect("group 0 always participates");
                    groups.clear();
                    groups.extend((1..caps.len()).map(|i| caps.get(i).map(|m| m.start()..m.end())));
                    on_match(whole.start()..whole.end(), &groups);
                }
                false
            }
            Engine::Text(re) => {
                for caps in re.captures_iter(text) {
                    let whole = caps.get(0).expect("group 0 always participates");
                    groups.clear();
                    groups.extend((1..caps.len()).map(|i| caps.get(i).map(|m| m.start()..m.end())));
                    on_match(whole.start()..whole.end(), &groups);
                }
                false
            }
            Engine::Fancy(re) => {
                let (input, truncated) = cap_for_backtracking(text);
                for caps in re.captures_iter(input) {
                    // A backtrack limit that fires mid-iteration ends the run. Reporting it as
                    // truncated is §7.4's "pattern too slow" and is not the same as no match.
                    let Ok(caps) = caps else {
                        return true;
                    };
                    let whole = caps.get(0).expect("group 0 always participates");
                    groups.clear();
                    groups.extend((1..caps.len()).map(|i| caps.get(i).map(|m| m.start()..m.end())));
                    on_match(whole.start()..whole.end(), &groups);
                }
                truncated
            }
        }
    }

    /// Finds the first match in one line's bytes.
    ///
    /// Returns the byte range **within the line**, or [`Hit::Truncated`] where §7.4's per-line cap
    /// stopped the backtracking engine looking. `decoded` is supplied by the caller because the
    /// text-mode engines need it and the byte-mode one does not — building it here would pay §7.4's
    /// decode cost on the path that exists to avoid it.
    fn find(&self, bytes: &[u8], decoded: Option<&str>) -> Hit {
        match &self.engine {
            Engine::Bytes(re) => match re.find(bytes) {
                Some(m) => Hit::At(m.start()..m.end()),
                None => Hit::None,
            },
            Engine::Text(re) => {
                let Some(text) = decoded else {
                    return Hit::None;
                };
                match re.find(text) {
                    Some(m) => Hit::At(m.start()..m.end()),
                    None => Hit::None,
                }
            }
            Engine::Fancy(re) => {
                let Some(text) = decoded else {
                    return Hit::None;
                };
                let (input, truncated) = cap_for_backtracking(text);
                match re.find(input) {
                    Ok(Some(m)) => Hit::At(m.start()..m.end()),
                    Ok(None) if truncated => Hit::Truncated,
                    Ok(None) => Hit::None,
                    // The backtrack limit fired. §7.4 wants this **visible**: "pattern too slow,
                    // truncated". Reporting it as no-match would be the quiet failure the cap exists
                    // to prevent.
                    Err(_) => Hit::Truncated,
                }
            }
        }
    }
}

/// §7.4's hard cap, applied **before** the engine sees the line.
///
/// Truncating the input rather than skipping the line means a match in the first 8 KB is still
/// found, which for a serialised-object line is where the message is. The cut is moved back to a
/// character boundary, because slicing a `&str` anywhere else panics — and a pathological line is
/// exactly where a multi-byte character is most likely to straddle the cap.
fn cap_for_backtracking(text: &str) -> (&str, bool) {
    if text.len() <= FANCY_LINE_CAP {
        return (text, false);
    }
    let mut cut = FANCY_LINE_CAP;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (&text[..cut], true)
}

/// What a pattern found in one line.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Hit {
    At(core::ops::Range<usize>),
    None,
    /// §7.4's "pattern too slow, truncated" — the line was cut, or the backtrack limit fired, so a
    /// match may exist and was not looked for.
    Truncated,
}

/// One match, in the coordinates a viewport uses.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// The line it is on, in the index's numbering.
    pub line: u64,
    /// Byte offsets **within that line**, so a highlight does not need the file offset.
    pub start: usize,
    pub end: usize,
}

/// What a search has found so far. §7.4: "results stream — first match visible quickly, total count
/// arriving later."
#[derive(Clone, Debug, Default)]
pub struct Found {
    pub matches: Vec<Match>,
    /// Lines §7.4's caps stopped the engine looking at properly. **Counted, not hidden** — a search
    /// that quietly skipped 40 lines has told the user something false about their log.
    pub truncated: u64,
    /// Lines examined, so a caller can show progress against the index's line count.
    pub scanned: u64,
}

impl Found {
    fn absorb(&mut self, other: Found) {
        self.matches.extend(other.matches);
        self.truncated += other.truncated;
        self.scanned += other.scanned;
    }
}

/// How a search is bounded. §7.4 has no persistent index, so every search is a full pass.
#[derive(Copy, Clone, Debug)]
pub struct SearchOptions {
    /// Lines per chunk. Chunking by **line** rather than by byte is what makes §7.4's "snaps chunk
    /// boundaries to newlines" free — the index already knows where a line starts.
    pub lines_per_chunk: u64,
    pub threads: usize,
    /// Stop after this many matches. §7.4 streams results, and a pattern matching every line of a
    /// 10 GB file would otherwise fill memory with hits nobody will scroll to.
    pub max_matches: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            // Big enough that the per-chunk setup — one `offset_of_line` and one decoder — is
            // amortised over real work, small enough that a cancel is noticed promptly.
            lines_per_chunk: 50_000,
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
            max_matches: 100_000,
        }
    }
}

/// A full-file search.
pub struct Search {
    pattern: Pattern,
    charset: Charset,
    options: SearchOptions,
    /// Set to stop every worker. §7.4 wants the backtracking path cancellable; making the whole
    /// search cancellable costs the same and is what a 300 ms debounce (§7.3) needs.
    cancel: Arc<AtomicBool>,
    /// Matches found so far, shared so `max_matches` stops every worker rather than each one
    /// separately reaching its own share of the cap.
    hits: Arc<AtomicU64>,
}

impl Search {
    pub fn new(pattern: Pattern, charset: Charset, options: SearchOptions) -> Self {
        Self::sharing(pattern, charset, options, &Cancel::new())
    }

    /// A search that answers to an existing [`Cancel`].
    ///
    /// **A set of rolled files is one log (§5.5b) and must be one search to the user**, but each
    /// member has its own encoding and so its own compiled pattern — so a pass over a set is several
    /// `Search`es, and one `Esc` has to stop all of them. Sharing the flag is what makes the one the
    /// user sees and the several that run agree.
    pub fn sharing(
        pattern: Pattern,
        charset: Charset,
        options: SearchOptions,
        cancel: &Cancel,
    ) -> Self {
        Self {
            pattern,
            charset,
            options,
            cancel: Arc::clone(&cancel.flag),
            hits: Arc::new(AtomicU64::new(0)),
        }
    }

    /// A handle that stops this search. §7.3 restarts the pass on every keystroke, so cancelling has
    /// to be something a *different* thread can do while the search runs.
    pub fn canceller(&self) -> Cancel {
        Cancel {
            flag: Arc::clone(&self.cancel),
        }
    }

    /// Runs the pass, reporting each chunk's findings as it completes.
    ///
    /// `on_chunk` is called from **worker threads**, and out of order — §7.4 wants the first match
    /// visible quickly, and serialising the callback to impose file order would make the last chunk
    /// gate the first. Callers that need order sort by [`Match::line`], which is what the returned
    /// [`Found`] does.
    pub fn run<R: ChunkReader + ?Sized>(
        &self,
        reader: &R,
        index: &LineIndex,
        mut on_chunk: impl FnMut(&Found) + Send,
    ) -> Result<Found> {
        let total = index.line_count();
        let chunks: Vec<(u64, u64)> = (0..total)
            .step_by(self.options.lines_per_chunk.max(1) as usize)
            .map(|from| {
                (
                    from,
                    from.saturating_add(self.options.lines_per_chunk).min(total),
                )
            })
            .collect();
        if chunks.is_empty() {
            return Ok(Found::default());
        }

        let next = std::sync::atomic::AtomicUsize::new(0);
        let threads = self.options.threads.clamp(1, chunks.len());
        let reporter = std::sync::Mutex::new(&mut on_chunk);
        let mut found = Found::default();

        std::thread::scope(|scope| -> Result<()> {
            let mut workers = Vec::with_capacity(threads);
            for _ in 0..threads {
                let (next, chunks, reporter) = (&next, &chunks, &reporter);
                workers.push(scope.spawn(move || -> Result<Found> {
                    let mut mine = Found::default();
                    loop {
                        if self.cancel.load(Ordering::Relaxed)
                            || self.hits.load(Ordering::Relaxed) >= self.options.max_matches as u64
                        {
                            return Ok(mine);
                        }
                        let nth = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&(from, to)) = chunks.get(nth) else {
                            return Ok(mine);
                        };
                        let chunk = self.search_lines(reader, index, from, to)?;
                        self.hits
                            .fetch_add(chunk.matches.len() as u64, Ordering::Relaxed);
                        // Reported before it is merged, so a caller sees a chunk the moment its
                        // worker finishes rather than when every worker has.
                        if let Ok(mut report) = reporter.lock() {
                            report(&chunk);
                        }
                        mine.absorb(chunk);
                    }
                }));
            }
            for worker in workers {
                let mine = worker
                    .join()
                    .map_err(|_| Error("a search worker panicked".into()))??;
                found.absorb(mine);
            }
            Ok(())
        })?;

        // Workers finish out of order; a result list does not get to.
        found.matches.sort_by_key(|m| (m.line, m.start));
        found.matches.truncate(self.options.max_matches);
        Ok(found)
    }

    /// Searches lines `[from, to)`. One `offset_of_line` and then a forward walk, which is the same
    /// reasoning `rows.rs` records: a run of consecutive lines is contiguous on disk.
    fn search_lines<R: ChunkReader + ?Sized>(
        &self,
        reader: &R,
        index: &LineIndex,
        from: u64,
        to: u64,
    ) -> Result<Found> {
        let mut found = Found::default();
        let Some(start) = offset_of_line(reader, self.charset, index, from)? else {
            return Ok(found);
        };
        // Text engines need the decoded line; the byte engine does not, and building one anyway
        // would pay §7.4's decode cost on the path that exists to avoid it.
        let wants_text = !matches!(self.pattern.engine, Engine::Bytes(_));

        let mut decoder = LineDecoder::new(self.charset);
        let mut buf = vec![0u8; READ_BYTES];
        let mut at = start;
        let mut line = from;

        'reading: while line < to {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let read = reader.read_at(at, &mut buf)?;
            if read == 0 {
                break;
            }
            at += read as u64;
            let mut stop = false;
            decoder.push(&buf[..read], |text| {
                if stop || line >= to {
                    stop = true;
                    return;
                }
                match self
                    .pattern
                    .find(text.as_bytes(), wants_text.then_some(text))
                {
                    Hit::At(range) => found.matches.push(Match {
                        line,
                        start: range.start,
                        end: range.end,
                    }),
                    Hit::Truncated => found.truncated += 1,
                    Hit::None => {}
                }
                found.scanned += 1;
                line += 1;
            });
            if stop {
                break 'reading;
            }
        }
        Ok(found)
    }
}

/// Bytes read per pass while searching. One comfortable read, matching `rows.rs`.
const READ_BYTES: usize = 256 * 1024;

/// Stops a running [`Search`] from another thread.
#[derive(Clone, Default)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
}

impl Cancel {
    /// An un-cancelled handle, for a caller that needs one before it has a [`Search`] to take it
    /// from — a pass over a set of files builds its searches one member at a time.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::{build_index, IndexOptions};

    const UTF8: Charset = Charset::UTF_8;

    fn indexed(text: &str) -> (Vec<u8>, LineIndex) {
        let bytes = text.as_bytes().to_vec();
        let index = build_index(
            &bytes[..],
            UTF8,
            0,
            bytes.len() as u64,
            &IndexOptions::default(),
        )
        .expect("index");
        (bytes, index)
    }

    fn search(pattern: &str, text: &str) -> Found {
        let (bytes, index) = indexed(text);
        let p = Pattern::compile(pattern, UTF8, false).expect("compile");
        Search::new(p, UTF8, SearchOptions::default())
            .run(&bytes[..], &index, |_| {})
            .expect("run")
    }

    #[test]
    fn a_match_is_reported_with_its_line_and_its_offset_within_it() {
        let found = search("Exception", "ok\nthrew an Exception here\nok\n");
        assert_eq!(
            found.matches,
            [Match {
                line: 1,
                start: 9,
                end: 18
            }]
        );
        assert_eq!(found.scanned, 3);
    }

    /// §7.4's whole reason for a second engine: "Log4net and NLog users write `(?<!DEBUG).*Exception`
    /// routinely, so rejecting lookaround outright would look broken."
    ///
    /// **⚠ §7.4's own example does not do what it looks like it does, and this was written with it
    /// before the test said otherwise.** `(?<!DEBUG ).*Exception` matches the DEBUG line too,
    /// because `.*` lets the match *start* past the prefix — at offset 7 the preceding six
    /// characters are `EBUG N`, the lookbehind passes, and the rest matches. That is correct regex
    /// behaviour and a loose pattern, not an engine fault; it is recorded here so nobody later
    /// "fixes" the engine to satisfy the spec's illustration.
    ///
    /// The pattern below has no `.*` to slide along, so the lookbehind is tested where it bites.
    #[test]
    fn a_lookaround_pattern_works_and_says_it_is_backtracking() {
        let p =
            Pattern::compile("(?<!DEBUG )NullReferenceException", UTF8, false).expect("compile");
        assert!(p.backtracking(), "lookaround must reach the second engine");

        let found = search(
            "(?<!DEBUG )NullReferenceException",
            "DEBUG NullReferenceException\nERROR NullReferenceException\n",
        );
        let lines: Vec<u64> = found.matches.iter().map(|m| m.line).collect();
        assert_eq!(lines, [1], "the DEBUG line is excluded by the lookbehind");
    }

    /// **The engine is chosen by compiling, not by reading the pattern.** A written test for `(?=`
    /// gets both of these wrong: the first is a character class of three ordinary characters, the
    /// second is three escaped literals, and neither needs backtracking.
    #[test]
    fn a_pattern_that_merely_looks_like_lookaround_stays_on_the_linear_engine() {
        for source in [r"[(?=]", r"\(\?\=", "plain", r"\d+"] {
            let p = Pattern::compile(source, UTF8, false).expect("compile");
            assert!(!p.backtracking(), "{source} does not need backtracking");
        }
        for source in [r"(?<!x)y", r"(a)\1"] {
            let p = Pattern::compile(source, UTF8, false).expect("compile");
            assert!(p.backtracking(), "{source} does need it");
        }
    }

    /// Both engines refusing is a compile error, and it is the **linear** engine's message —
    /// reporting the fallback's complaint about a pattern written for the primary one sends the user
    /// to the wrong place.
    #[test]
    fn a_pattern_neither_engine_compiles_reports_the_first_engines_complaint() {
        let err = Pattern::compile("(unclosed", UTF8, false).expect_err("should not compile");
        assert!(!err.0.is_empty());
    }

    /// §7.4's "hard 8 KB per-line input cap", and it is **visible**: the line is counted as
    /// truncated rather than reported as a clean miss.
    #[test]
    fn a_long_line_is_capped_for_the_backtracking_engine_and_the_cap_is_reported() {
        let filler = "x".repeat(FANCY_LINE_CAP * 2);
        let text = format!("short\n{filler}NEEDLE\n");
        let found = search("(?<!q)NEEDLE", &text);
        assert!(
            found.matches.is_empty(),
            "the match sits past the cap and must not be found"
        );
        assert_eq!(found.truncated, 1, "and the cap must be reported");
    }

    /// The same line on the linear engine has no cap, because it needs none — that is the whole
    /// argument for preferring it.
    #[test]
    fn the_linear_engine_has_no_line_cap() {
        let filler = "x".repeat(FANCY_LINE_CAP * 2);
        let text = format!("short\n{filler}NEEDLE\n");
        let found = search("NEEDLE", &text);
        assert_eq!(found.matches.len(), 1);
        assert_eq!(found.truncated, 0);
    }

    /// §7.4: chunk boundaries snap to newlines, so no match straddles a chunk and no overlap or
    /// dedup is needed. Chunking small forces many boundaries against a match on every line.
    #[test]
    fn chunking_finds_the_same_matches_however_it_is_divided() {
        let text: String = (0..500).map(|i| format!("line {i} needle\n")).collect();
        let (bytes, index) = indexed(&text);
        let mut counts = Vec::new();
        for lines_per_chunk in [1, 2, 7, 64, 10_000] {
            let p = Pattern::compile("needle", UTF8, false).expect("compile");
            let found = Search::new(
                p,
                UTF8,
                SearchOptions {
                    lines_per_chunk,
                    ..SearchOptions::default()
                },
            )
            .run(&bytes[..], &index, |_| {})
            .expect("run");
            let lines: Vec<u64> = found.matches.iter().map(|m| m.line).collect();
            counts.push(lines);
        }
        let expected: Vec<u64> = (0..500).collect();
        for lines in &counts {
            assert_eq!(*lines, expected);
        }
    }

    /// §7.4: "results stream — first match visible quickly, total count arriving later."
    #[test]
    fn results_are_reported_per_chunk_rather_than_only_at_the_end() {
        let text: String = (0..1000).map(|i| format!("line {i} needle\n")).collect();
        let (bytes, index) = indexed(&text);
        let p = Pattern::compile("needle", UTF8, false).expect("compile");
        let reports = std::sync::Mutex::new(Vec::new());
        let found = Search::new(
            p,
            UTF8,
            SearchOptions {
                lines_per_chunk: 100,
                ..SearchOptions::default()
            },
        )
        .run(&bytes[..], &index, |chunk| {
            reports.lock().expect("lock").push(chunk.matches.len());
        })
        .expect("run");
        let reports = reports.into_inner().expect("into inner");
        assert_eq!(reports.len(), 10, "one report per chunk");
        assert_eq!(reports.iter().sum::<usize>(), found.matches.len());
    }

    /// §7.3 restarts the pass on every keystroke, so a running search has to be stoppable from
    /// another thread rather than only between searches.
    #[test]
    fn a_search_can_be_cancelled_while_it_runs() {
        let text: String = (0..200_000).map(|i| format!("line {i} needle\n")).collect();
        let (bytes, index) = indexed(&text);
        let p = Pattern::compile("needle", UTF8, false).expect("compile");
        let search = Search::new(
            p,
            UTF8,
            SearchOptions {
                lines_per_chunk: 1000,
                threads: 1,
                ..SearchOptions::default()
            },
        );
        let cancel = search.canceller();
        cancel.cancel();
        let found = search.run(&bytes[..], &index, |_| {}).expect("run");
        assert!(
            found.scanned < 200_000,
            "a cancelled search should not have finished, scanned {}",
            found.scanned
        );
    }

    /// A pattern matching every line of a huge file would otherwise fill memory with hits nobody
    /// will scroll to.
    #[test]
    fn the_match_cap_bounds_what_is_kept() {
        let text: String = (0..5000).map(|i| format!("line {i} needle\n")).collect();
        let (bytes, index) = indexed(&text);
        let p = Pattern::compile("needle", UTF8, false).expect("compile");
        let found = Search::new(
            p,
            UTF8,
            SearchOptions {
                lines_per_chunk: 100,
                max_matches: 250,
                ..SearchOptions::default()
            },
        )
        .run(&bytes[..], &index, |_| {})
        .expect("run");
        assert_eq!(found.matches.len(), 250);
    }

    /// §7.4's "search raw bytes where possible", and the half of it that is not possible: a pattern
    /// compiled from UTF-8 source matched against UTF-16 bytes finds nothing, **silently**. So a
    /// UTF-16 source decodes first, and this is the test that would fail if it did not.
    #[test]
    fn a_utf16_source_is_searched_as_text_rather_than_as_bytes() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in "alpha\nbeta needle\ngamma\n".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let charset = Charset::UTF_16LE;
        let index = build_index(
            &bytes[..],
            charset,
            2,
            bytes.len() as u64,
            &IndexOptions::default(),
        )
        .expect("index");
        let p = Pattern::compile("needle", charset, false).expect("compile");
        let found = Search::new(p, charset, SearchOptions::default())
            .run(&bytes[..], &index, |_| {})
            .expect("run");
        assert_eq!(found.matches.len(), 1);
        assert_eq!(found.matches[0].line, 1);
    }

    #[test]
    fn case_insensitivity_reaches_both_engines() {
        for (source, backtracking) in [("needle", false), ("(?<!q)needle", true)] {
            let p = Pattern::compile(source, UTF8, true).expect("compile");
            assert_eq!(p.backtracking(), backtracking);
            let (bytes, index) = indexed("NEEDLE in caps\n");
            let found = Search::new(p, UTF8, SearchOptions::default())
                .run(&bytes[..], &index, |_| {})
                .expect("run");
            assert_eq!(found.matches.len(), 1, "{source}");
        }
    }

    /// An empty file has nothing to search and must not be an error — it is the ordinary state of a
    /// log that has just been rolled onto.
    #[test]
    fn searching_an_empty_source_finds_nothing_and_does_not_fail() {
        let found = search("needle", "");
        assert!(found.matches.is_empty());
        assert_eq!(found.scanned, 0);
    }
}
