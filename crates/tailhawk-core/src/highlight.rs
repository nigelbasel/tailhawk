//! Highlight rules — `SPEC.md` §7.1: what colour a run of characters is, and who decides.
//!
//! §7.1: "Rules are **plain text or regex**, each with foreground colour, background colour, and a
//! whole-line-vs-match-only toggle. Regex capture groups may be sub-highlighted." They "live in
//! named, **importable/exportable** sets … per-rule enable/disable", and a set "can be **bound to a
//! file glob or a detected format**, so opening a matching file applies it automatically. **No
//! incumbent does this.**"
//!
//! ## Precedence is list order, and that is a decision §7.1 leaves open
//!
//! §7.1 says rules have precedence and does not say what the order is. **The first rule in the set
//! to claim a character keeps it** — so precedence is the order shown in the UI, and reordering the
//! list is how a user changes it.
//!
//! The alternatives are all invisible: longest-match, most-specific and last-wins each produce an
//! ordering the user cannot see in the list they are looking at, so a rule that quietly loses gives
//! them nothing to act on. List order is the only rule that explains itself.
//!
//! **Capture groups are the deliberate exception.** A group's colour sits *above* its own rule's,
//! because §7.1 asks for sub-highlighting and a sub-highlight that lost to its parent would never
//! show. It is done by claiming the groups **before** the match that contains them — under
//! first-claim-wins that is all "above" means, and it needs no second priority concept.
//!
//! ## Visible rows only, and a budget on top of that
//!
//! §7.1: "Highlights are computed **for visible rows only**, never for every ingested line." That is
//! a rule about *where* this is called from, and `paint.rs` is the only caller.
//!
//! It is not enough on its own. §10.3 supports lines up to 32 KB, and a screenful of them against a
//! set of thirty rules is a megabyte of regex work inside a 16.67 ms frame. So [`Highlighter::line`]
//! also carries a byte budget: rows past it get **no spans and draw plain**, which is a visibly
//! duller frame rather than a late one — §11.3's rule, applied to colour instead of to text.
//!
//! **Bytes are not the only cost, and assuming they were was a bug.** The work is per *match*, and a
//! 32 KB line against a rule as ordinary as `\w` produces 32,000 of them — inside the byte budget
//! and nowhere near inside a frame. [`MAX_SPANS_PER_LINE`] is the second bound, and [`claim`] keeps
//! its list sorted so an insertion is a binary search rather than a scan. See both for the
//! measurement that forced them.
//!
//! ## Derived colours
//!
//! §7.1: "stable derived colours for recurring identifiers — the same request ID is the same colour
//! everywhere, in every file." A [`Rule`] with [`Rule::derived`] set takes its foreground from the
//! **text it matched** rather than from a fixed colour: the text is hashed and the hash picks from
//! [`IDENTIFIER_PALETTE`]. Nothing is remembered between lines, frames or files, which is what makes
//! it stable — there is no table to be in a different state tomorrow.
//!
//! ## What is not here
//!
//! - **The zero-config semantic layer** of §7.1 is `semantic.rs` — a catalogue of rules for this
//!   engine, sitting *beneath* user rules.
//! - **Glob and format binding.** [`RuleSet::bound_to`] carries the string so a set can be
//!   round-tripped, and nothing matches on it yet — format detection is M6.
//! - **Import and export.** The types are plain data and derive what a serialiser would need, but no
//!   file format is chosen; picking one now would be picking it without the settings model (§12).

use core::cell::Cell;

use crate::search::Pattern;

/// Bytes of line text one frame will examine before it stops highlighting.
///
/// **A frame budget, not a correctness bound.** §10.3's 32 KB lines times a screenful is far more
/// regex work than a 16.67 ms frame holds, and the failure this prevents is a *late* frame rather
/// than a wrong one.
///
/// **Sized from a measurement, on 2026-08-17.** It was 256 KB — "roughly eight full-width 32 KB
/// lines" — chosen before there was a rule set to time. `semantic.rs`'s tripwire test then
/// measured its 21-rule catalogue at ~11.9 µs for a 150-byte row in release, about 80 ns per byte:
/// 256 KB was ~21 ms of highlighting, over the frame the budget exists to protect. 64 KB is ~5 ms
/// with the catalogue alone (a user's rules go on top), two full-width lines, and still four to
/// eight screenfuls of ordinary 100–200 byte rows.
pub const FRAME_BUDGET_BYTES: usize = 64 * 1024;

/// Spans one line will produce before the highlighter stops adding more.
///
/// **Bounded because the cost is per *match*, not per byte, and the frame budget only counts bytes.**
/// A 32 KB line -- §10.3's limit -- against a rule like `\w` yields 32,000 matches, and no screen
/// shows 32,000 coloured runs of one row: §10.3 caps the *rendered* part of a line long before that.
/// Past this the row keeps the colours it has, which is the same "duller rather than later" trade the
/// frame budget makes.
pub const MAX_SPANS_PER_LINE: usize = 4096;

/// A colour, in the same linear-ish sRGB the renderer takes.
pub type Colour = [f32; 4];

/// The colours a [derived](Rule::derived) rule draws from, indexed by the hash of the matched text.
///
/// **Eight, spread round the hue circle at one lightness**, so that two identifiers a reader is
/// comparing are as likely as the palette allows to differ in hue rather than in brightness — the
/// question a derived colour answers is "is this the same one?", and same-hue-different-lightness
/// reads as "probably". All are legible on `BACKGROUND` and none is the severity ramp's red, amber
/// or magenta, so an identifier never impersonates an error. Provisional with the rest of the
/// palette (`UI-DESIGN.md` §11.2 pins no hex).
pub const IDENTIFIER_PALETTE: [Colour; 8] = [
    [0.55, 0.78, 0.98, 1.0],
    [0.62, 0.86, 0.66, 1.0],
    [0.86, 0.72, 0.98, 1.0],
    [0.55, 0.88, 0.86, 1.0],
    [0.92, 0.80, 0.55, 1.0],
    [0.98, 0.68, 0.78, 1.0],
    [0.72, 0.80, 0.98, 1.0],
    [0.80, 0.90, 0.55, 1.0],
];

/// The colour a derived rule gives `text`. Same text, same colour, everywhere and always.
///
/// FNV-1a over the bytes, folded onto the palette. The hash is not chosen for quality — eight
/// buckets need no more than "not obviously correlated with the input" — but for being fully
/// specified, so the answer cannot change with a standard-library or crate release.
pub fn derived_colour(text: &str) -> Colour {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    IDENTIFIER_PALETTE[(hash % IDENTIFIER_PALETTE.len() as u64) as usize]
}

/// One highlight rule. §7.1's unit.
pub struct Rule {
    /// What the user called it. Shown in the rule list and used by nothing else.
    pub name: String,
    pub pattern: Pattern,
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
    /// §7.1's "stable derived colours for recurring identifiers": the foreground comes from
    /// [`derived_colour`] of the matched text, and `fg` is ignored. See the module note.
    pub derived: bool,
    /// §7.1's "whole-line-vs-match-only toggle". Whole-line still requires a match — it changes what
    /// the match *covers*, not whether the rule fires.
    pub whole_line: bool,
    /// §7.1's per-rule enable/disable. A disabled rule keeps its place in the order, so re-enabling
    /// it restores the precedence it had rather than moving it to the end.
    pub enabled: bool,
    /// §7.1's "regex capture groups may be sub-highlighted": one entry per group, in group order.
    /// `None` leaves the group taking its rule's colours.
    pub groups: Vec<Option<Colour>>,
}

impl Rule {
    /// A rule that colours whatever it matches.
    pub fn new(name: impl Into<String>, pattern: Pattern) -> Self {
        Self {
            name: name.into(),
            pattern,
            fg: None,
            bg: None,
            derived: false,
            whole_line: false,
            enabled: true,
            groups: Vec::new(),
        }
    }

    pub fn fg(mut self, colour: Colour) -> Self {
        self.fg = Some(colour);
        self
    }

    /// Colours each match by its own text — §7.1's identifier colours.
    pub fn derived(mut self) -> Self {
        self.derived = true;
        self
    }

    pub fn bg(mut self, colour: Colour) -> Self {
        self.bg = Some(colour);
        self
    }

    pub fn whole_line(mut self) -> Self {
        self.whole_line = true;
        self
    }

    /// Gives capture group `n` (1-based, as written in the pattern) its own colour.
    pub fn group(mut self, n: usize, fg: Colour) -> Self {
        if self.groups.len() < n {
            self.groups.resize(n, None);
        }
        self.groups[n - 1] = Some(fg);
        self
    }
}

/// A named, ordered set of rules. §7.1's importable/exportable unit.
pub struct RuleSet {
    pub name: String,
    /// **In precedence order**, highest first. See the module note.
    pub rules: Vec<Rule>,
    /// §7.1's "bound to a file glob or a detected format". Carried, not yet matched on.
    pub bound_to: Option<String>,
}

impl RuleSet {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rules: Vec::new(),
            bound_to: None,
        }
    }

    pub fn with(mut self, rule: Rule) -> Self {
        self.rules.push(rule);
        self
    }
}

/// A coloured run of one line, in byte offsets within it.
///
/// Spans come back **sorted and non-overlapping**, because that is what a painter can consume in one
/// pass over the row. Resolving overlaps here rather than there is the whole job of this module.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
}

/// Applies a rule set to lines, under §7.1's visible-rows rule and a frame budget.
///
/// **The budget is interior-mutable and [`line`](Self::line) takes `&self`**, because the caller
/// is [`RowSource::row_spans`](crate::rows::RowSource::row_spans), which the painter reaches
/// through `&dyn RowSource` — a shared borrow, for the whole frame, of the source that owns this.
/// The alternative was a `RefCell` in every source that highlights, or a `&mut dyn RowSource`
/// through the painter for the sake of two counters; a `Cell` for each counter is the smaller
/// change and it is honest about what mutates: the frame's accounting, never the rules.
pub struct Highlighter {
    set: RuleSet,
    /// Bytes left in this frame. Reset by [`begin_frame`](Self::begin_frame).
    budget: Cell<usize>,
    /// Rows this frame gave up on, so a caller can say so rather than leave them looking unmatched.
    skipped: Cell<u32>,
}

impl Highlighter {
    pub fn new(set: RuleSet) -> Self {
        Self {
            set,
            budget: Cell::new(FRAME_BUDGET_BYTES),
            skipped: Cell::new(0),
        }
    }

    pub fn set(&self) -> &RuleSet {
        &self.set
    }

    pub fn set_mut(&mut self) -> &mut RuleSet {
        &mut self.set
    }

    /// Starts a frame's budget. Called once per frame, before any row.
    pub fn begin_frame(&self) {
        self.budget.set(FRAME_BUDGET_BYTES);
        self.skipped.set(0);
    }

    /// Rows this frame ran out of budget for. They draw plain; §11.3 prefers that to a late frame.
    pub fn skipped(&self) -> u32 {
        self.skipped.get()
    }

    /// The spans for one line, sorted and non-overlapping.
    ///
    /// `out` is cleared and reused, so a painter holds one `Vec` for the whole frame instead of
    /// allocating per row — at fifty rows a frame that is fifty allocations a frame for nothing.
    pub fn line(&self, line: &str, out: &mut Vec<Span>) {
        out.clear();
        self.beneath(line, out);
    }

    /// The spans for one line, **added beneath whatever `out` already holds.**
    ///
    /// This is how the shell layers a search's matches over the semantic catalogue: the matches
    /// go into `out` first, and this fills in around them under the same first-claim-wins rule
    /// that orders the set — so a match the user asked for is never hidden by a timestamp colour,
    /// and everything a match does not cover is still coloured. `out` must be sorted and
    /// non-overlapping on entry, which is what every producer of spans in this crate emits.
    pub fn beneath(&self, line: &str, out: &mut Vec<Span>) {
        let budget = self.budget.get();
        if line.len() > budget {
            // **Charged before the work, so an over-budget row costs nothing.** Charging afterwards
            // would let one 32 KB line blow the budget it was supposed to be stopped by.
            self.skipped.set(self.skipped.get() + 1);
            self.budget.set(0);
            return;
        }
        self.budget.set(budget - line.len());

        for rule in self.set.rules.iter().filter(|r| r.enabled) {
            rule.pattern.each_match(line, |whole, groups| {
                // **Groups first.** Under first-claim-wins that is the whole of "a sub-highlight
                // sits above its rule"; see the module note.
                for (n, range) in groups.iter().enumerate() {
                    let (Some(range), Some(Some(fg))) = (range, rule.groups.get(n)) else {
                        continue;
                    };
                    claim(
                        out,
                        Span {
                            start: range.start,
                            end: range.end,
                            fg: Some(*fg),
                            bg: rule.bg,
                        },
                    );
                }
                let (start, end) = if rule.whole_line {
                    (0, line.len())
                } else {
                    (whole.start, whole.end)
                };
                // Derived from the *match*, not the whole line, so a whole-line identifier rule
                // still colours by the identifier it found.
                let fg = if rule.derived {
                    Some(derived_colour(&line[whole.start..whole.end]))
                } else {
                    rule.fg
                };
                // **A rule with no colours of its own claims only its groups.** That is how a
                // rule anchors a group on context — `status=(\d{3})` colouring the code — without
                // taking `status=` away from a later rule that wanted it: the linear engine has no
                // lookbehind, so the context has to be *in* the match, and claiming it in no
                // colour would block for nothing.
                if fg.is_none() && rule.bg.is_none() {
                    return;
                }
                claim(
                    out,
                    Span {
                        start,
                        end,
                        fg,
                        bg: rule.bg,
                    },
                );
            });
        }
    }
}

/// Adds `span`, keeping only the parts no earlier span already claimed.
///
/// **`out` is kept sorted and non-overlapping at all times, and that is a performance requirement
/// rather than tidiness.** The first version scanned the whole of `out` per insertion, which is
/// O(n²) in *matches per line* — and §10.3 allows a 32 KB line, so a rule as ordinary as `\w`
/// produces 32,000 matches and a billion comparisons for one row. A test that built a 170 KB line
/// took **over sixty seconds**, which is how it was found; the slowness was the defect, not the test.
///
/// Sorted, the overlapping run is a `partition_point` plus a walk over just the spans that actually
/// overlap. Insertion memmoves, but a memmove of a few thousand 40-byte spans is not what a frame's
/// time goes on, and [`MAX_SPANS_PER_LINE`] bounds it anyway.
fn claim(out: &mut Vec<Span>, span: Span) {
    if span.start >= span.end || out.len() >= MAX_SPANS_PER_LINE {
        return;
    }
    // The first span that could overlap. `out` is non-overlapping and sorted by `start`, so `end`
    // is sorted too and this is a binary search.
    let first = out.partition_point(|s| s.end <= span.start);
    let mut at = span.start;
    let mut insert_at = first;
    let mut additions: Vec<Span> = Vec::new();

    let mut i = first;
    while i < out.len() && out[i].start < span.end {
        if out[i].start > at {
            additions.push(Span {
                start: at,
                end: out[i].start,
                ..span
            });
        }
        at = at.max(out[i].end);
        i += 1;
    }
    if at < span.end {
        additions.push(Span {
            start: at,
            end: span.end,
            ..span
        });
    }

    // Spliced in one go, in order, so the invariant holds at every observable point.
    for addition in additions {
        while insert_at < out.len() && out[insert_at].start < addition.start {
            insert_at += 1;
        }
        if out.len() >= MAX_SPANS_PER_LINE {
            return;
        }
        out.insert(insert_at, addition);
        insert_at += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Charset;

    const UTF8: Charset = Charset::UTF_8;
    const RED: Colour = [1.0, 0.0, 0.0, 1.0];
    const GREEN: Colour = [0.0, 1.0, 0.0, 1.0];
    const BLUE: Colour = [0.0, 0.0, 1.0, 1.0];

    fn rule(name: &str, pattern: &str) -> Rule {
        Rule::new(
            name,
            Pattern::compile(pattern, UTF8, false).expect("compile"),
        )
    }

    fn spans(set: RuleSet, line: &str) -> Vec<Span> {
        let h = Highlighter::new(set);
        h.begin_frame();
        let mut out = Vec::new();
        h.line(line, &mut out);
        out
    }

    fn ranges(spans: &[Span]) -> Vec<(usize, usize, Option<Colour>)> {
        spans.iter().map(|s| (s.start, s.end, s.fg)).collect()
    }

    #[test]
    fn a_rule_colours_what_it_matches() {
        let set = RuleSet::new("s").with(rule("err", "ERROR").fg(RED));
        assert_eq!(
            ranges(&spans(set, "12:00 ERROR failed")),
            [(6, 11, Some(RED))]
        );
    }

    /// §7.1's whole-line toggle: the match decides *whether*, not *what*.
    #[test]
    fn a_whole_line_rule_covers_the_line_but_still_needs_a_match() {
        let set = RuleSet::new("s").with(rule("err", "ERROR").fg(RED).whole_line());
        let line = "12:00 ERROR failed";
        assert_eq!(ranges(&spans(set, line)), [(0, line.len(), Some(RED))]);

        let set = RuleSet::new("s").with(rule("err", "ERROR").fg(RED).whole_line());
        assert!(spans(set, "12:00 INFO fine").is_empty());
    }

    /// **Precedence is list order** — the decision §7.1 leaves open. The first rule to claim a
    /// character keeps it, so the second rule colours only what is left.
    #[test]
    fn the_first_rule_in_the_list_wins_the_characters_it_claims() {
        let set = RuleSet::new("s")
            .with(rule("word", "ERROR").fg(RED))
            .with(rule("line", ".*").fg(GREEN));
        assert_eq!(
            ranges(&spans(set, "a ERROR b")),
            [(0, 2, Some(GREEN)), (2, 7, Some(RED)), (7, 9, Some(GREEN))]
        );
    }

    /// The same two rules the other way round give the other answer, which is the point: the order
    /// the user sees is the order that decides.
    #[test]
    fn reordering_the_list_reorders_the_precedence() {
        let set = RuleSet::new("s")
            .with(rule("line", ".*").fg(GREEN))
            .with(rule("word", "ERROR").fg(RED));
        assert_eq!(ranges(&spans(set, "a ERROR b")), [(0, 9, Some(GREEN))]);
    }

    /// §7.1: "regex capture groups may be sub-highlighted." A group's colour sits **above** its own
    /// rule's, or a sub-highlight could never show.
    #[test]
    fn a_capture_group_outranks_the_rule_that_found_it() {
        let set = RuleSet::new("s").with(
            rule("kv", r"(\w+)=(\w+)")
                .fg(GREEN)
                .group(1, RED)
                .group(2, BLUE),
        );
        assert_eq!(
            ranges(&spans(set, "user=nigel")),
            [(0, 4, Some(RED)), (4, 5, Some(GREEN)), (5, 10, Some(BLUE))]
        );
    }

    /// …but the group still loses to an **earlier rule**, because precedence is between rules and
    /// the group exception is only within one.
    #[test]
    fn a_capture_group_still_loses_to_an_earlier_rule() {
        let set = RuleSet::new("s")
            .with(rule("user", "user").fg(GREEN))
            .with(rule("kv", r"(\w+)=(\w+)").group(1, RED).group(2, BLUE));
        let out = spans(set, "user=nigel");
        assert_eq!(
            out[0],
            Span {
                start: 0,
                end: 4,
                fg: Some(GREEN),
                bg: None
            }
        );
    }

    /// §7.1's per-rule enable/disable, and the part worth testing: a disabled rule **keeps its
    /// place**, so re-enabling it restores the precedence it had.
    #[test]
    fn disabling_a_rule_does_not_move_it() {
        let mut h = Highlighter::new(
            RuleSet::new("s")
                .with(rule("word", "ERROR").fg(RED))
                .with(rule("line", ".*").fg(GREEN)),
        );
        h.set_mut().rules[0].enabled = false;
        h.begin_frame();
        let mut out = Vec::new();
        h.line("a ERROR b", &mut out);
        assert_eq!(ranges(&out), [(0, 9, Some(GREEN))]);

        h.set_mut().rules[0].enabled = true;
        h.begin_frame();
        h.line("a ERROR b", &mut out);
        assert_eq!(
            ranges(&out),
            [(0, 2, Some(GREEN)), (2, 7, Some(RED)), (7, 9, Some(GREEN))],
            "re-enabling must restore the original precedence, not append it"
        );
    }

    /// Spans reach the painter sorted and non-overlapping, because a painter consumes a row in one
    /// pass and cannot be asked to resolve overlaps itself.
    #[test]
    fn spans_come_back_sorted_and_never_overlapping() {
        let set = RuleSet::new("s")
            .with(rule("digits", r"\d+").fg(RED))
            .with(rule("words", r"\w+").fg(GREEN))
            .with(rule("all", ".").fg(BLUE));
        let out = spans(set, "ab 12 cd 34 ef");
        for pair in out.windows(2) {
            assert!(pair[0].end <= pair[1].start, "{pair:?} overlap or unsorted");
        }
        // And they tile the line exactly, because the last rule matches every character.
        assert_eq!(out.first().map(|s| s.start), Some(0));
        assert_eq!(out.last().map(|s| s.end), Some(14));
    }

    /// §11.3's rule applied to colour: a row past the frame budget draws **plain**, which is a
    /// duller frame rather than a late one — and it is counted, not hidden.
    #[test]
    fn a_row_past_the_frame_budget_draws_plain_and_is_counted() {
        let long = "x".repeat(FRAME_BUDGET_BYTES + 1);
        let h = Highlighter::new(RuleSet::new("s").with(rule("x", "x").fg(RED)));
        h.begin_frame();
        let mut out = Vec::new();
        h.line(&long, &mut out);
        assert!(out.is_empty());
        assert_eq!(h.skipped(), 1);

        // A new frame starts with a full budget, or one bad row would poison every frame after it.
        h.begin_frame();
        h.line("short x", &mut out);
        assert!(!out.is_empty());
        assert_eq!(h.skipped(), 0);
    }

    /// The budget is charged **before** the work, so the row that exceeds it does not also do it.
    #[test]
    fn an_over_budget_row_is_not_examined_before_being_skipped() {
        let h = Highlighter::new(RuleSet::new("s").with(rule("x", "x").fg(RED)));
        h.begin_frame();
        let mut out = Vec::new();
        // Two rows that individually fit and together do not.
        let each = "x".repeat(FRAME_BUDGET_BYTES * 2 / 3);
        h.line(&each, &mut out);
        assert!(!out.is_empty(), "the first fits");
        h.line(&each, &mut out);
        assert!(out.is_empty(), "the second must not be examined");
        assert_eq!(h.skipped(), 1);
    }

    /// **A §10.3-sized line against a rule that matches nearly every character.** This is the shape
    /// that made the first version take over a minute: the cost is per match, the byte budget only
    /// counts bytes, and `claim` scanned its whole list per insertion.
    ///
    /// The assertion is the span cap rather than a duration — a timing assertion would flake on a
    /// loaded machine. What holds the performance is that the *suite* runs in under a second; the
    /// quadratic version could not.
    #[test]
    fn a_dense_rule_on_a_full_width_line_is_bounded_rather_than_quadratic() {
        // §10.3's supported line length, every character a word character.
        let line = "a".repeat(32 * 1024);
        let set = RuleSet::new("s").with(rule("each", r"\w").fg(RED));
        let out = spans(set, &line);
        assert_eq!(out.len(), MAX_SPANS_PER_LINE);
        for pair in out.windows(2) {
            assert!(pair[0].end <= pair[1].start);
        }
    }

    /// Reaching the cap must not corrupt what is already there — the row keeps the colours it has
    /// and simply stops gaining more.
    #[test]
    fn spans_stay_sorted_and_disjoint_right_up_to_the_cap() {
        let line = "ab ".repeat(20_000);
        let set = RuleSet::new("s")
            .with(rule("a", "a").fg(RED))
            .with(rule("b", "b").fg(GREEN))
            .with(rule("space", " ").fg(BLUE));
        let out = spans(set, &line);
        assert!(out.len() <= MAX_SPANS_PER_LINE);
        for pair in out.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "{:?} then {:?} overlap or are unsorted",
                pair[0],
                pair[1]
            );
        }
    }

    /// An empty set is not a broken one — it is the state before the user has written a rule, and
    /// §7.1's whole complaint about klogg is that this state should not look like a failure.
    #[test]
    fn an_empty_rule_set_produces_no_spans_and_no_error() {
        assert!(spans(RuleSet::new("empty"), "a line").is_empty());
    }

    /// A rule matching nothing on this line contributes nothing, rather than a zero-width span the
    /// painter would have to filter.
    #[test]
    fn a_zero_width_match_is_not_a_span() {
        let set = RuleSet::new("s").with(rule("empty", "x*").fg(RED));
        let out = spans(set, "abc");
        for span in &out {
            assert!(span.end > span.start, "{span:?} is empty");
        }
    }

    /// Highlighting reuses `search.rs`'s engine policy, so §7.4's escape hatch is available here
    /// too — and a rule is exactly where a log4net user writes one.
    #[test]
    fn a_lookaround_rule_works_because_the_engine_policy_is_shared() {
        let set = RuleSet::new("s").with(rule("not-debug", "(?<!DEBUG )Exception").fg(RED));
        assert!(spans(
            RuleSet::new("s").with(rule("not-debug", "(?<!DEBUG )Exception").fg(RED)),
            "DEBUG Exception"
        )
        .is_empty());
        assert_eq!(ranges(&spans(set, "ERROR Exception")), [(6, 15, Some(RED))]);
    }

    /// §7.1: "the same request ID is the same colour everywhere, in every file". Two matches of
    /// the same text agree, on one line and across lines, and a fixed `fg` on the rule is ignored.
    #[test]
    fn a_derived_rule_gives_the_same_text_the_same_colour_everywhere() {
        let set = RuleSet::new("s").with(rule("id", "req-[0-9a-f]+").fg(RED).derived());
        let h = Highlighter::new(set);
        let mut a = Vec::new();
        let mut b = Vec::new();
        h.line("req-1a2b then req-9f then req-1a2b", &mut a);
        h.line("later: req-1a2b", &mut b);

        assert_eq!(a.len(), 3);
        assert_eq!(a[0].fg, a[2].fg, "same text, same colour on one line");
        assert_eq!(a[0].fg, b[0].fg, "same text, same colour on another line");
        assert_ne!(
            a[0].fg,
            Some(RED),
            "a derived rule ignores its fixed foreground"
        );
        assert!(IDENTIFIER_PALETTE.contains(&a[0].fg.expect("coloured")));
        assert_eq!(a[0].fg, Some(derived_colour("req-1a2b")));
    }

    /// The palette is only useful if different identifiers usually differ: eight buckets cannot
    /// separate every pair, but a run of distinct ids should not collapse onto one or two colours.
    #[test]
    fn derived_colours_spread_across_the_palette() {
        let mut seen = std::collections::HashSet::new();
        for i in 0..64 {
            let colour = derived_colour(&format!("0HN7{i:04X}"));
            seen.insert(colour.map(f32::to_bits));
        }
        assert_eq!(
            seen.len(),
            IDENTIFIER_PALETTE.len(),
            "64 ids should reach every bucket"
        );
    }

    /// The shell puts a search's matches into `out` first and calls `beneath` for the rest, so a
    /// match keeps its colour where the rules would have claimed the same characters, and the rules
    /// still colour everything the match does not cover.
    #[test]
    fn beneath_fills_around_what_is_already_claimed_and_never_over_it() {
        let set = RuleSet::new("s").with(rule("word", "[a-z]+").fg(RED));
        let h = Highlighter::new(set);
        h.begin_frame();
        let mut out = vec![Span {
            start: 4,
            end: 8,
            fg: None,
            bg: Some(BLUE),
        }];
        h.beneath("abc defgh ij", &mut out);
        assert_eq!(
            out.iter()
                .map(|s| (s.start, s.end, s.fg, s.bg))
                .collect::<Vec<_>>(),
            [
                (0, 3, Some(RED), None),
                (4, 8, None, Some(BLUE)),
                (8, 9, Some(RED), None),
                (10, 12, Some(RED), None),
            ]
        );
    }

    /// A colourless rule with a coloured group claims the group and nothing else, so the context
    /// it matched on stays available to the rules below it.
    #[test]
    fn a_colourless_rule_claims_only_its_groups() {
        let set = RuleSet::new("s")
            .with(rule("code", r"status=(\d{3})").group(1, RED))
            .with(rule("key", r"([a-z]+)=").group(1, GREEN));
        assert_eq!(
            ranges(&spans(set, "status=503")),
            [(0, 6, Some(GREEN)), (7, 10, Some(RED))]
        );
    }

    /// `line` and `beneath` share one budget, and `line` is `beneath` onto an emptied `out`.
    #[test]
    fn line_is_beneath_onto_nothing_and_shares_the_frame_budget() {
        let set = RuleSet::new("s").with(rule("word", "[a-z]+").fg(RED));
        let h = Highlighter::new(set);
        h.begin_frame();
        let mut out = vec![Span {
            start: 0,
            end: 1,
            fg: None,
            bg: Some(BLUE),
        }];
        h.line("abc", &mut out);
        assert_eq!(ranges(&out), [(0, 3, Some(RED))], "line starts from empty");

        let long = "x".repeat(FRAME_BUDGET_BYTES);
        h.beneath(&long, &mut out);
        assert_eq!(
            h.skipped(),
            1,
            "the budget `line` spent is the budget `beneath` sees"
        );
    }
}
