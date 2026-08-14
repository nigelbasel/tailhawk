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
//! ## What is not here
//!
//! - **The zero-config semantic layer** of §7.1 — timestamps, GUIDs, IPs, paths, `key=value`. That
//!   is E23, it sits *beneath* user rules, and it is a catalogue rather than an engine.
//! - **Stable derived colours for recurring identifiers** ("the same request ID is the same colour
//!   everywhere"). §7.1 wants it; it needs a hash-to-palette and a palette, and it belongs with E23.
//! - **Glob and format binding.** [`RuleSet::bound_to`] carries the string so a set can be
//!   round-tripped, and nothing matches on it yet — format detection is M6.
//! - **Import and export.** The types are plain data and derive what a serialiser would need, but no
//!   file format is chosen; picking one now would be picking it without the settings model (§12).

use crate::search::Pattern;

/// Bytes of line text one frame will examine before it stops highlighting.
///
/// **A frame budget, not a correctness bound.** §10.3's 32 KB lines times a screenful is far more
/// regex work than a 16.67 ms frame holds, and the failure this prevents is a *late* frame rather
/// than a wrong one. 256 KB is roughly eight full-width 32 KB lines, or a screenful of ordinary
/// ones several times over.
pub const FRAME_BUDGET_BYTES: usize = 256 * 1024;

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

/// One highlight rule. §7.1's unit.
pub struct Rule {
    /// What the user called it. Shown in the rule list and used by nothing else.
    pub name: String,
    pub pattern: Pattern,
    pub fg: Option<Colour>,
    pub bg: Option<Colour>,
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
            whole_line: false,
            enabled: true,
            groups: Vec::new(),
        }
    }

    pub fn fg(mut self, colour: Colour) -> Self {
        self.fg = Some(colour);
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
pub struct Highlighter {
    set: RuleSet,
    /// Bytes left in this frame. Reset by [`begin_frame`](Self::begin_frame).
    budget: usize,
    /// Rows this frame gave up on, so a caller can say so rather than leave them looking unmatched.
    skipped: u32,
}

impl Highlighter {
    pub fn new(set: RuleSet) -> Self {
        Self {
            set,
            budget: FRAME_BUDGET_BYTES,
            skipped: 0,
        }
    }

    pub fn set(&self) -> &RuleSet {
        &self.set
    }

    pub fn set_mut(&mut self) -> &mut RuleSet {
        &mut self.set
    }

    /// Starts a frame's budget. Called once per frame by the painter, before any row.
    pub fn begin_frame(&mut self) {
        self.budget = FRAME_BUDGET_BYTES;
        self.skipped = 0;
    }

    /// Rows this frame ran out of budget for. They draw plain; §11.3 prefers that to a late frame.
    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    /// The spans for one line, sorted and non-overlapping.
    ///
    /// `out` is cleared and reused, so a painter holds one `Vec` for the whole frame instead of
    /// allocating per row — at fifty rows a frame that is fifty allocations a frame for nothing.
    pub fn line(&mut self, line: &str, out: &mut Vec<Span>) {
        out.clear();
        if line.len() > self.budget {
            // **Charged before the work, so an over-budget row costs nothing.** Charging afterwards
            // would let one 32 KB line blow the budget it was supposed to be stopped by.
            self.skipped += 1;
            self.budget = 0;
            return;
        }
        self.budget -= line.len();

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
                claim(
                    out,
                    Span {
                        start,
                        end,
                        fg: rule.fg,
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
        let mut h = Highlighter::new(set);
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
        let mut h = Highlighter::new(RuleSet::new("s").with(rule("x", "x").fg(RED)));
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
        let mut h = Highlighter::new(RuleSet::new("s").with(rule("x", "x").fg(RED)));
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
}
