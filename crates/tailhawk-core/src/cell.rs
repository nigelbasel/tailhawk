//! The cell model — V4, to `SPEC.md` §3.3.
//!
//! The naive "one character, one cell" assumption breaks on real log content and §3.3 rejects it.
//! The unit is the **grapheme cluster**: East Asian Wide and Fullwidth clusters take two cells,
//! combining marks add none, and a ZWJ emoji sequence is one cluster however many code points it
//! contains.
//!
//! **Width comes from `unicode-width`'s cluster-aware `UnicodeWidthStr::width`, applied one cluster
//! at a time.** The tempting alternative — take the width of the cluster's *base* character and
//! ignore the rest — was written first and is wrong in at least four ways that a review caught:
//! multi-jamo Hangul (`ᄀ가` is one cluster of four cells, not two), Devanagari and Thai **spacing**
//! marks (`कि` is two cells, not one — §3.3's "combining marks occupy 0 cells" is true of `Mn`/`Me`
//! marks, not of `Mc`), and a stray `U+FE0F` or `U+FE0E` anywhere in a line, which is
//! attacker-controllable (§13.4) and shifted every column to its right.
//!
//! Summing *code point* widths is also wrong — `👨‍👩‍👧‍👦` would be eight cells for something that draws
//! in two — but `UnicodeWidthStr::width` does not do that: it segments and handles ZWJ sequences,
//! regional-indicator pairs, keycaps and presentation selectors itself. Delegating to it deleted
//! three hand-written special cases that were each subtly wrong.
//!
//! Nothing here rasterises or measures a font. This is the grid's arithmetic, and it is deliberately
//! portable and testable without a device: §3.3's acceptance test is about *columns lining up*, which
//! is decided here, not in DirectWrite. Where a fallback font's advance disagrees with the primary,
//! §3.3 is explicit that **the cell grid wins** and the glyph is centred — so this module is the
//! authority and the renderer follows it.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Variation selector-16, "render the preceding character as emoji".
///
/// Only meaningful on a base with `Emoji=Yes`; anywhere else it is a defective sequence with no
/// effect on width, which is why it is **not** a rule on its own.
const EMOJI_PRESENTATION: char = '\u{FE0F}';

/// Emoji skin-tone modifiers, `U+1F3FB`–`U+1F3FF`.
fn is_emoji_modifier(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// How the model treats characters that normally draw nothing.
///
/// `SPEC.md` §13.4 specifies a **"reveal invisibles"** toggle that renders bidi controls,
/// zero-width characters and other `Cf` code points visibly. That is a width question as much as a
/// painting one — a revealed zero-width space occupies a cell — so it belongs here.
///
/// **⚠ The toggle is incomplete, and §13.4 cannot yet be claimed.** It reveals an invisible that
/// forms its **own** grapheme cluster — `U+200B`, and the `U+202E` override of the Trojan Source
/// attack §13.4 names, which is the case that matters most. It does **not** reveal one absorbed into
/// a preceding cluster: `a` + `U+200D`, `a` + a tag character (`U+E0067`, the hidden-text vector),
/// or `a` + a variation selector are all one cluster whose width is unchanged, so the toggle does
/// nothing. Fixing that needs `General_Category` data to tell a `Cf` character from a legitimate
/// combining mark — which must not be revealed — and probably needs reveal mode to segment
/// differently rather than only to measure differently. Recorded rather than papered over:
/// `revealing_does_not_yet_reach_an_invisible_inside_a_cluster` asserts the gap so it stays visible.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct CellModel {
    /// §13.4's toggle. Off by default: a zero-width character is zero cells wide, as it is
    /// everywhere else.
    pub reveal_invisibles: bool,
}

/// Clusters between sampled anchors. See [`ColumnAnchors`].
///
/// 64 to match [`ANCHOR_STRIDE`](crate::index::ANCHOR_STRIDE), because it is the same trade-off one
/// axis over and there is no reason for the two to disagree. A 32 KB line of two-byte clusters
/// samples 256 anchors, or 4 KB — against the 32 KB of text it describes.
pub const COLUMN_ANCHOR_STRIDE: usize = 64;

/// Sampled `(byte, cell)` pairs through one line, so a column lookup does not start from byte zero.
///
/// **This is `SPEC.md` §5.3's line index, transposed onto the column axis**, and it is here for the
/// same reason: the walk it replaces was measured, in the shipped binary, at **76 ms a frame** with
/// the viewport at the end of a 19.4 KB line containing one non-ASCII character — against a 16.67 ms
/// budget. `view.rs` asks for a byte span and a starting column once per row per frame, and both
/// walked `grapheme_indices` from the front of the line.
///
/// An anchor is a *hint that is always safe*: every lookup is exact whatever anchors it is given —
/// none, any stride, or a set built for a **different line**. [`ColumnAnchors::none`] is a legitimate
/// argument everywhere and only costs speed, which is what keeps a stale or absent anchor set from
/// ever being a correctness question.
///
/// **That last clause was earned rather than assumed.** An adversarial review of this change
/// reproduced two ways it could have been false: slicing from a foreign anchor's byte offset panicked
/// out of a `pub fn` (`start byte index 384 is out of bounds for string of length 18`), and a foreign
/// anchor that happened to land on a valid boundary resumed at a wrong column — which for
/// `byte_span` is silent §5.6 content loss and strictly worse than the crash. Both were refuted as
/// live bugs, correctly, because `Rows` builds `anchors[i]` from `lines[i]` and `paint.rs` pairs them
/// in one loop iteration. They are closed anyway, by the `line_len` check below, because a promise
/// this module makes in writing should be true of the code and not only of its callers.
///
/// **Not built for a line on the [`is_column_per_byte`](CellModel::is_column_per_byte) fast path**,
/// because there the mapping is already O(1) and anchors would be pure overhead.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColumnAnchors {
    /// `(byte, cell)` at cluster 0, `STRIDE`, `2·STRIDE`, … Ascending in both fields, which is what
    /// makes the binary searches below valid.
    marks: Vec<(u32, u32)>,
    /// Byte length of the line these were built from.
    ///
    /// **The anchors are ignored for a line of any other length**, which is what makes this type's
    /// promise — a wrong or stale set costs speed, never correctness — true rather than nearly true.
    /// Without it, anchors from another line either panic on an out-of-range slice or, worse, land
    /// on a valid boundary of the new line and resume at a **wrong column**: for `byte_span` that is
    /// §5.6 content loss, silent, and strictly worse than a crash.
    ///
    /// It does not prove identity — two different lines of equal length would still match — and it
    /// is not meant to. `Rows` builds `anchors[i]` from `lines[i]` and `paint.rs` pairs them in one
    /// loop iteration, so identity is structural; this closes the accidental mismatch that a future
    /// caller could introduce, at the cost of one `u32`.
    line_len: u32,
}

impl ColumnAnchors {
    /// No anchors — every lookup still exact, just from byte zero.
    pub const fn none() -> Self {
        Self {
            marks: Vec::new(),
            line_len: 0,
        }
    }

    /// A borrowable empty set, so a caller with nothing to offer can still satisfy a `&ColumnAnchors`
    /// without allocating or holding one.
    pub fn none_ref() -> &'static Self {
        static NONE: ColumnAnchors = ColumnAnchors::none();
        &NONE
    }

    /// Samples `line`, or returns [`none`](Self::none) for a line that does not need it.
    ///
    /// One walk, done where the row's text is decoded rather than in the frame — which is the whole
    /// point: `Rows` fetches a row once and paints it many times while the horizontal offset moves.
    pub fn build(model: &CellModel, line: &str) -> Self {
        if CellModel::is_column_per_byte(line) || line.len() <= COLUMN_ANCHOR_STRIDE {
            return Self::none();
        }
        Self::build_with_stride(model, line, COLUMN_ANCHOR_STRIDE)
    }

    /// The sampling itself, with the stride exposed so tests can drive it at 1, 2 and 3 — where
    /// every cluster is a boundary case and the resume logic is stressed far harder than 64 ever
    /// stresses it on a short fixture.
    fn build_with_stride(model: &CellModel, line: &str, stride: usize) -> Self {
        let stride = stride.max(1);
        let mut marks = Vec::with_capacity(line.len() / stride + 1);
        for (i, c) in model.walk(line).enumerate() {
            if i % stride == 0 {
                // A line past 4 GB cannot be reached: §10.3 caps the rendered extent far below it
                // and `view.rs` caps the bytes handed to the shaper at 8 KB.
                let (Ok(byte), Ok(cell)) = (u32::try_from(c.byte), u32::try_from(c.cell)) else {
                    break;
                };
                marks.push((byte, cell));
            }
        }
        Self {
            marks,
            line_len: u32::try_from(line.len()).unwrap_or(0),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Whether these anchors describe this line at all. See the `line_len` field.
    fn fits(&self, line: &str) -> bool {
        !self.marks.is_empty() && usize::try_from(self.line_len) == Ok(line.len())
    }

    /// Where to resume a byte-driven walk: an anchor if these describe `line`, the front otherwise.
    fn start_for_byte(&self, line: &str, byte: usize) -> (usize, usize) {
        if self.fits(line) {
            self.before_byte(byte)
        } else {
            (0, 0)
        }
    }

    /// Where to resume a column-driven walk. Same rule.
    fn start_for_cell(&self, line: &str, cell: usize) -> (usize, usize) {
        if self.fits(line) {
            self.before_cell(cell)
        } else {
            (0, 0)
        }
    }

    /// The last anchor at or before `byte`, as `(byte, cell)`.
    fn before_byte(&self, byte: usize) -> (usize, usize) {
        let i = self.marks.partition_point(|(b, _)| (*b as usize) <= byte);
        match i.checked_sub(1) {
            Some(i) => (self.marks[i].0 as usize, self.marks[i].1 as usize),
            None => (0, 0),
        }
    }

    /// The last anchor at or before `cell`, as `(byte, cell)`.
    ///
    /// **`<` on the cell, not `<=`, and the difference is §5.6 rather than an off-by-one.**
    /// Zero-width clusters occupy no column, so several anchors can share one; `<=` lands on the
    /// *last* of them and the walk then begins after the ones before it. `an_anchor_never_changes_
    /// an_answer` catches it on `"\u{202E}abc"` — the Trojan Source line §13.4 names — where
    /// `byte_span(0..1)` returns `3..4` instead of `0..4` and **silently drops the attacker-supplied
    /// bidi override from the copied bytes**. That is exactly the loss `byte_span`'s outward
    /// rounding exists to prevent, reintroduced through a binary search. Starting before every
    /// anchor at that column keeps them all inside the walk.
    fn before_cell(&self, cell: usize) -> (usize, usize) {
        let i = self.marks.partition_point(|(_, c)| (*c as usize) < cell);
        match i.checked_sub(1) {
            Some(i) => (self.marks[i].0 as usize, self.marks[i].1 as usize),
            None => (0, 0),
        }
    }
}

/// One grapheme cluster, placed on the grid.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// Byte offset of the cluster within the line.
    pub byte: usize,
    /// Byte length of the cluster.
    pub byte_len: usize,
    /// Cell column the cluster starts at.
    pub cell: usize,
    /// Cells it occupies: 0, 1 or 2.
    pub width: usize,
}

impl CellModel {
    pub const fn new() -> Self {
        Self {
            reveal_invisibles: false,
        }
    }

    pub const fn revealing() -> Self {
        Self {
            reveal_invisibles: true,
        }
    }

    /// Cells occupied by one grapheme cluster.
    ///
    /// Three rules, in order:
    ///
    /// 1. **A control character is one cell, always.** §5.6 requires control bytes and NULs to
    ///    render as a visible replacement glyph and never be silently dropped, and `unicode-width`
    ///    gives them zero. `CR LF` is one cluster and so one cell.
    /// 2. **An emoji modifier sequence is two cells.** This is the one place `UnicodeWidthStr` is
    ///    worse: it counts `👍🏻` as 2 + 2 and `☝️🏻` as 2 + 2, where both draw as a single two-cell
    ///    glyph. Guarded on the base actually being emoji, so `A` followed by a stray modifier still
    ///    counts as the two separate things it draws as.
    /// 3. **Otherwise `unicode-width`'s cluster-aware answer**, which already handles ZWJ sequences,
    ///    regional-indicator pairs, keycaps, presentation selectors, conjoining jamo and spacing
    ///    marks — every one of which a hand-written rule got wrong.
    ///
    /// A zero-width result stays zero unless §13.4's reveal toggle is on. See [`CellModel`] for what
    /// that toggle does **not** yet cover.
    /// Whether this cluster occupies a cell **only because** the reveal toggle is on — a
    /// zero-width cluster (a bidi override, a joiner, a zero-width space) that §13.4 wants shown.
    ///
    /// The painter draws a marker in that cell instead of the cluster's glyphs: the glyphs of an
    /// invisible carry a full advance and would land on the next character, and an empty cell says
    /// nothing. Byte offsets are untouched — this is a question about drawing, not about text.
    pub fn is_revealed(&self, cluster: &str) -> bool {
        self.reveal_invisibles
            && cluster.chars().next().is_some_and(|c| c.width().is_some())
            && cluster.width() == 0
    }

    pub fn cluster_width(&self, cluster: &str) -> usize {
        let Some(base) = cluster.chars().next() else {
            return 0;
        };

        if base.width().is_none() {
            return 1;
        }

        if cluster.chars().any(is_emoji_modifier)
            && (base.width() == Some(2) || cluster.contains(EMOJI_PRESENTATION))
        {
            return 2;
        }

        match cluster.width() {
            0 if self.reveal_invisibles => 1,
            width => width,
        }
    }

    /// Whether this line's cell columns **are** its byte offsets, so the cluster walk can be skipped.
    ///
    /// **This is the fix for the horizontal scroll cost, and it is an identity rather than an
    /// approximation.** Every function below maps between bytes and columns by walking
    /// `grapheme_indices` from byte zero, which `view.rs` calls once per row per frame — measured in
    /// the shipped binary at **76 ms a frame** with the viewport at the end of a 19.4 KB line,
    /// against 16 ms at column 0. §10.3 puts exactly those lines in scope.
    ///
    /// For a line of ASCII with no `\n` the walk cannot tell you anything the byte offset does not:
    ///
    /// - every ASCII byte is its own grapheme cluster — the sole ASCII exception is `CR LF`, which
    ///   is one cluster, and a line cannot contain `\n`, which is why that is the guard;
    /// - every ASCII cluster is **one cell wide**: printable ASCII is width 1, and a control
    ///   character is width 1 by [`cluster_width`](Self::cluster_width)'s first rule — which fires
    ///   *before* the zero-width check, so this holds under §13.4's reveal toggle as well as
    ///   without it. That is what makes the fast path independent of `reveal_invisibles`.
    ///
    /// So column `n` is byte `n`, exactly, and `a_line_of_ascii_agrees_with_the_full_walk` asserts
    /// that against the general path rather than taking the argument's word for it.
    ///
    /// Both checks are vectorised byte scans over a line that is about to be shaped anyway, so this
    /// replaces two O(clusters) walks with two O(bytes) passes that run at memory speed.
    pub fn is_column_per_byte(line: &str) -> bool {
        line.is_ascii() && !line.as_bytes().contains(&b'\n')
    }

    /// Every cluster in `line`, in order.
    pub fn cells<'a>(&'a self, line: &'a str) -> impl Iterator<Item = Cell> + 'a {
        let mut cell = 0usize;
        line.grapheme_indices(true).map(move |(byte, cluster)| {
            let width = self.cluster_width(cluster);
            let placed = Cell {
                byte,
                byte_len: cluster.len(),
                cell,
                width,
            };
            cell += width;
            placed
        })
    }

    /// Whether the ASCII byte at `p` is, on its own, a whole grapheme cluster of width 1.
    ///
    /// **A purely local test, and that is what makes the fast walk possible.** Between two ASCII
    /// characters UAX #29 breaks in every case but one — GB3's `CR × LF` — so a byte whose
    /// neighbours are both ASCII is its own cluster, and every single-ASCII-character cluster is one
    /// cell (printable by width, control by [`cluster_width`](Self::cluster_width)'s first rule,
    /// which fires before the zero-width check and so holds under §13.4's reveal toggle too).
    ///
    /// **Both neighbours are checked, and both matter.** The byte *after* could be a combining mark
    /// that absorbs this one — `a` + `U+0301` is one cluster, not two. The byte *before* could be a
    /// `Prepend` (GB9b), which absorbs the character that follows it; those are Indic and Arabic
    /// code points, so a non-ASCII predecessor disqualifies the byte and an ASCII one cannot.
    fn ascii_singleton(bytes: &[u8], p: usize) -> bool {
        const CR: u8 = b'\r';
        const LF: u8 = b'\n';
        if bytes[p] >= 0x80 {
            return false;
        }
        let before_ok = p == 0 || (bytes[p - 1] < 0x80 && !(bytes[p - 1] == CR && bytes[p] == LF));
        let after_ok = p + 1 == bytes.len()
            || (bytes[p + 1] < 0x80 && !(bytes[p] == CR && bytes[p + 1] == LF));
        before_ok && after_ok
    }

    /// Every cluster in `line`, the same sequence [`cells`](Self::cells) produces, reached faster.
    ///
    /// **This exists because of a measurement, and it replaces nothing semantically.** Building a
    /// row's [`ColumnAnchors`] is one full walk of the line, and with the viewport scrolled right
    /// that dominated a frame: 44 ms for 48 rows of 19.4 KB. The cost is `grapheme_indices` plus a
    /// `unicode-width` lookup **per cluster**, ~19,400 of each per row — and for a log line, all but
    /// a handful of those clusters are one ASCII byte whose answer is known without asking.
    ///
    /// So a byte satisfying [`ascii_singleton`](Self::ascii_singleton) is emitted directly, and
    /// anything else is handed to real segmentation. The fallback runs from the current position to
    /// the next byte that *is* a singleton, and both ends of that span are true cluster boundaries —
    /// the near end by induction, the far end because a singleton is preceded by ASCII — which is
    /// what makes segmenting the substring in isolation sound. See
    /// [`cells_from`](Self::cells_from) for the same argument about regional indicators.
    ///
    /// `the_fast_walk_and_the_plain_one_agree` is the differential test, and it is the only reason
    /// to believe any of this: hand-written rules about text in this module have a history of being
    /// wrong in four ways at once.
    fn walk<'a>(&'a self, line: &'a str) -> Box<dyn Iterator<Item = Cell> + 'a> {
        // **Chosen per line, because the fast path is a bet that most clusters are ASCII.**
        //
        // When that bet loses it loses twice: an adversarial review measured the fallback on a line
        // with no ASCII singleton anywhere — all-CJK, or any non-ASCII/ASCII alternation, where
        // every ASCII byte has a non-ASCII neighbour — at **26% slower than the plain walk**, and
        // fixing the memory half of that left it still 18% slower, because segmenting one cluster
        // at a time builds a fresh iterator per cluster where `cells` carries one cursor. There is
        // no per-byte repair for that; the decision has to be made before starting.
        //
        // Counting the non-ASCII bytes is a vectorised scan — ~0.3 ms against a 68 ms walk on 3 MB,
        // so the probe is free at the scale where the answer matters. A quarter is a threshold
        // rather than a measurement: a cluster is a singleton only if **both** neighbours are ASCII,
        // so the singleton count collapses well before the byte count does, and the two shapes this
        // has to separate — a log line with one `—` in it, and CJK interleaved with ASCII — sit at
        // opposite ends of it.
        let bytes = line.as_bytes();
        let non_ascii = bytes.iter().filter(|b| **b >= 0x80).count();
        if non_ascii * 4 > bytes.len() {
            return Box::new(self.cells(line));
        }
        Box::new(self.walk_ascii_first(line))
    }

    /// [`walk`](Self::walk)'s fast path, for a line that is predominantly ASCII.
    fn walk_ascii_first<'a>(&'a self, line: &'a str) -> impl Iterator<Item = Cell> + 'a {
        let bytes = line.as_bytes();
        let mut p = 0usize;
        let mut cell = 0usize;

        std::iter::from_fn(move || {
            if p >= bytes.len() {
                return None;
            }
            if Self::ascii_singleton(bytes, p) {
                let placed = Cell {
                    byte: p,
                    byte_len: 1,
                    cell,
                    width: 1,
                };
                p += 1;
                cell += 1;
                return Some(placed);
            }
            // **One cluster, segmented where it starts** — `p` is always a cluster boundary, so the
            // first grapheme of `line[p..]` is the right one and nothing before `p` can change it.
            //
            // An earlier version segmented ahead to the next singleton and buffered the whole span
            // into a `Vec`. An adversarial review measured what that cost on a line with no
            // singleton anywhere — all-CJK, or any non-ASCII/ASCII alternation — where the span is
            // the entire line: **~10–16× the line's bytes held transiently, and 26% slower than the
            // plain walk it replaced** (67.2 ms against 53.2 ms on 3 MB). That is a pure regression
            // on exactly the shape §10.3 names as the one klogg hangs "deadly" on. The batching was
            // never needed: taking one cluster at a time is O(1) in memory and leaves the fast path
            // untouched.
            let cluster = line[p..]
                .graphemes(true)
                .next()
                .expect("p is inside the line");
            let width = self.cluster_width(cluster);
            let placed = Cell {
                byte: p,
                byte_len: cluster.len(),
                cell,
                width,
            };
            p += cluster.len();
            cell += width;
            Some(placed)
        })
    }

    /// Every cluster from a known cluster boundary, numbered as if walked from the front.
    ///
    /// **Resuming mid-line is sound only because `at_byte` is a true cluster boundary**, and that is
    /// worth stating because grapheme segmentation is not context-free. UAX #29's regional-indicator
    /// rules (GB12/GB13) pair flags by counting the run of preceding RIs, so resuming at an arbitrary
    /// byte could pair `🇦🇧🇨` differently from a full walk. It cannot happen here: an anchor is placed
    /// *at* a boundary the full walk produced, so every earlier RI has already been consumed into a
    /// complete cluster and the count restarts exactly as it did. The same holds for GB9b's prepend
    /// and GB11's ZWJ sequences — a cluster boundary means there is no pending context to carry.
    ///
    /// **An anchor that does not fit this line is ignored rather than trusted**, and that is what
    /// makes the promise in [`ColumnAnchors`] literally true instead of nearly true. Slicing
    /// `line[at_byte..]` with a byte from a *different* line panics — an adversarial review
    /// reproduced exactly that, `start byte index 384 is out of bounds for string of length 18`,
    /// through the public `byte_span_anchored`. It is unreachable today because `Rows` builds
    /// `anchors[i]` from `lines[i]` and `paint.rs` pairs them in one loop iteration, so the review
    /// refuted it as a live bug — correctly. But "a stale or absent anchor set is never a
    /// correctness question, only a slower one" is a claim this module makes in writing, and a
    /// panic out of a `pub fn` is not a slower answer. Falling back to the front of the line is.
    fn cells_from<'a>(
        &'a self,
        line: &'a str,
        at_byte: usize,
        at_cell: usize,
    ) -> impl Iterator<Item = Cell> + 'a {
        let (at_byte, at_cell) = if at_byte <= line.len() && line.is_char_boundary(at_byte) {
            (at_byte, at_cell)
        } else {
            (0, 0)
        };
        let mut cell = at_cell;
        line[at_byte..]
            .grapheme_indices(true)
            .map(move |(byte, cluster)| {
                let width = self.cluster_width(cluster);
                let placed = Cell {
                    byte: at_byte + byte,
                    byte_len: cluster.len(),
                    cell,
                    width,
                };
                cell += width;
                placed
            })
    }

    /// Total cells the line occupies — its horizontal extent.
    pub fn cell_count(&self, line: &str) -> usize {
        if Self::is_column_per_byte(line) {
            return line.len();
        }
        self.cells(line).map(|c| c.width).sum()
    }

    /// The cluster containing a byte offset, for turning a byte position into a column.
    ///
    /// A byte in the middle of a cluster resolves to that cluster, not to the next one.
    pub fn cell_at_byte(&self, line: &str, byte: usize) -> usize {
        self.cell_at_byte_anchored(line, byte, &ColumnAnchors::none())
    }

    /// [`cell_at_byte`](Self::cell_at_byte), starting from the nearest anchor at or before `byte`.
    pub fn cell_at_byte_anchored(&self, line: &str, byte: usize, anchors: &ColumnAnchors) -> usize {
        if Self::is_column_per_byte(line) {
            return byte.min(line.len());
        }
        let (from_byte, from_cell) = anchors.start_for_byte(line, byte);
        let mut last = from_cell;
        for cluster in self.cells_from(line, from_byte, from_cell) {
            if byte < cluster.byte {
                return last;
            }
            if byte < cluster.byte + cluster.byte_len {
                return cluster.cell;
            }
            last = cluster.cell + cluster.width;
        }
        last
    }

    /// The byte offset a cell column lands in — hit-testing a click, and clamping a selection.
    ///
    /// **A click on the second half of a wide cluster returns the cluster's start**, never a byte
    /// inside it. Selecting half a CJK character is not a thing a user can mean, and a byte offset
    /// that splits a cluster would be one the decoder cannot honour.
    ///
    /// **Zero-width clusters are skipped**, because they occupy no column and so cannot be clicked.
    /// Giving one a phantom column — which an earlier version did — means a click on the first
    /// visible character returns the offset of an invisible one *before* it, and the caret lands
    /// where the user did not click. `"\u{202E}abc"`, the Trojan Source line §13.4 names, is exactly
    /// that shape: the override is cluster 0 at column 0, and so is `a`.
    pub fn byte_at_cell(&self, line: &str, cell: usize) -> usize {
        self.byte_at_cell_anchored(line, cell, &ColumnAnchors::none())
    }

    /// [`byte_at_cell`](Self::byte_at_cell), starting from the nearest anchor at or before `cell`.
    pub fn byte_at_cell_anchored(&self, line: &str, cell: usize, anchors: &ColumnAnchors) -> usize {
        if Self::is_column_per_byte(line) {
            return cell.min(line.len());
        }
        let (from_byte, from_cell) = anchors.start_for_cell(line, cell);
        for cluster in self.cells_from(line, from_byte, from_cell) {
            if cluster.width > 0 && cell < cluster.cell + cluster.width {
                return cluster.byte;
            }
        }
        line.len()
    }

    /// The bytes a half-open **range** of cell columns covers — what a selection copies.
    ///
    /// **This is not `byte_at_cell(start)..byte_at_cell(end)`, and the difference is §5.6.**
    /// `byte_at_cell` skips zero-width clusters because they cannot be clicked, and composing two of
    /// them therefore **drops them from the copied bytes**. `"\u{202E}abc"` — the Trojan Source line
    /// §13.4 names — puts the override and `a` both at column 0, so selecting the whole visible line
    /// would yield `abc` and silently discard the attacker-supplied override. §5.6 forbids discarding
    /// content silently, and a copy that launders a bidi override is the worst version of it: the
    /// user pastes something that reads differently from what they selected.
    ///
    /// So the two ends round **outwards**, not the same way:
    ///
    /// - the **start** takes the *lowest* byte at that column, pulling in zero-width clusters sitting
    ///   there;
    /// - the **end** takes the *highest*, pulling in zero-width clusters trailing the last visible
    ///   one.
    ///
    /// **A zero-width cluster on an interior boundary therefore belongs to both neighbours**, and
    /// that is deliberate: two adjacent selections copy it twice rather than neither. Duplicating an
    /// invisible character is a cosmetic wrong answer; losing one changes what the text means.
    ///
    /// A column landing inside a wide cluster takes the whole cluster at either end, for the same
    /// reason [`CellModel::byte_at_cell`] does — half a CJK character is not a thing to select, and
    /// the byte offset that would express it is one the decoder cannot honour.
    ///
    /// **An empty column range is empty in bytes too**, and that needs saying because the outward
    /// rounding above would otherwise contradict it: a zero-width cluster sitting exactly on the
    /// caret's column satisfies *both* ends of a `c..c` range, so a caret would have copied the bidi
    /// override next to it. A caret is not a selection and copies nothing.
    pub fn byte_span(&self, line: &str, cells: core::ops::Range<usize>) -> core::ops::Range<usize> {
        self.byte_span_anchored(line, cells, &ColumnAnchors::none())
    }

    /// [`byte_span`](Self::byte_span), starting from the nearest anchor at or before `cells.start`.
    ///
    /// **This is the call the frame budget turns on.** `view.rs` makes it once per row per frame,
    /// and walking from byte zero measured 76 ms a frame at the end of a 19.4 KB line.
    pub fn byte_span_anchored(
        &self,
        line: &str,
        cells: core::ops::Range<usize>,
        anchors: &ColumnAnchors,
    ) -> core::ops::Range<usize> {
        if cells.start >= cells.end {
            let at = self.byte_at_cell_anchored(line, cells.start, anchors);
            return at..at;
        }
        if Self::is_column_per_byte(line) {
            // No zero-width clusters exist here, so the outward rounding the general path performs
            // has nothing to round outwards to and the two ends are just the clamped offsets.
            return cells.start.min(line.len())..cells.end.min(line.len());
        }
        let (from_byte, from_cell) = anchors.start_for_cell(line, cells.start);
        let mut start = None;
        let mut end = None;
        for cluster in self.cells_from(line, from_byte, from_cell) {
            // **Stop once the columns are behind us.** Clusters come in increasing column order, so
            // nothing at a column past `cells.end` can widen either bound — and without this the
            // loop walks every cluster in the line whatever the range asked for. That was free
            // while this was called once per copy; `view.rs` calls it once per row per frame, and
            // 50 rows of 32 KB lines measured 190 ms a frame against a 16.67 ms budget, at
            // horizontal offset *zero*. The condition is `>` rather than `>=` because a zero-width
            // cluster sitting exactly on `cells.end` is still inside, by the outward rounding above.
            if start.is_some() && cluster.cell > cells.end {
                break;
            }
            let starts_here = cluster.cell + cluster.width > cells.start
                || (cluster.width == 0 && cluster.cell >= cells.start);
            if start.is_none() && starts_here {
                start = Some(cluster.byte);
            }
            let inside =
                cluster.cell < cells.end || (cluster.width == 0 && cluster.cell == cells.end);
            if start.is_some() && inside {
                end = Some(cluster.byte + cluster.byte_len);
            }
        }
        let start = start.unwrap_or(line.len());
        start..end.unwrap_or(start).max(start)
    }

    /// The cell columns of the word around a column — what a double-click selects.
    ///
    /// **Word boundaries are UAX #29's**, taken from the same segmenter as the grapheme clusters, so
    /// `192.168.1.1` and `foo_bar` stay whole while `foo-bar` and `2026-08-06` split at the hyphens.
    /// That last pair is the one a log reader will notice, and it is a deliberate cost: the
    /// alternative is a hand-written character-class table, and this module's history is that every
    /// hand-written rule about text turned out to be wrong in at least four ways. A log-aware
    /// granularity — one that treats a timestamp or a path as a unit — is a separate, later thing,
    /// not a tweak to this.
    ///
    /// A column past the end of the line selects nothing there rather than the last word: clicking
    /// the empty space to the right of a short line means the empty space.
    pub fn word_at_cell(&self, line: &str, cell: usize) -> core::ops::Range<usize> {
        let byte = self.byte_at_cell(line, cell);
        if byte >= line.len() {
            let end = self.cell_count(line);
            return end..end;
        }
        for (at, word) in line.split_word_bound_indices() {
            if byte < at + word.len() {
                return self.cell_at_byte(line, at)..self.cell_at_byte(line, at + word.len());
            }
        }
        let end = self.cell_count(line);
        end..end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zero-width joiner. Not part of the width rules — a ZWJ cluster gets its width from its base
    /// like any other — but the fixtures need it to assert they are genuinely joined.
    const ZWJ: char = '\u{200D}';

    fn width(text: &str) -> usize {
        CellModel::new().cell_count(text)
    }

    /// **Every ASCII character is one cell, in both models — all 128, not a sample.**
    ///
    /// This is the load-bearing half of the fast walk's argument: `ascii_singleton` emits width 1
    /// without consulting `cluster_width` at all, so a single ASCII character measuring anything
    /// else would make the two walks disagree silently and only for that byte. Printable ASCII is
    /// obvious; the ones worth enumerating are `NUL`, `DEL` and the C0 controls, which reach width 1
    /// through `cluster_width`'s **first** rule — and that rule fires before the zero-width branch,
    /// which is what makes the claim hold under §13.4's reveal toggle too.
    ///
    /// Written by an adversarial reviewer as a throwaway probe and kept, because it establishes by
    /// enumeration what the surrounding argument merely asserts.
    #[test]
    fn every_ascii_character_is_one_cell_in_both_models() {
        for b in 0u8..=0x7f {
            let s = (b as char).to_string();
            assert_eq!(
                (
                    CellModel::new().cluster_width(&s),
                    CellModel::revealing().cluster_width(&s)
                ),
                (1, 1),
                "ASCII {b:#04x} is not one cell in both models"
            );
        }
    }

    /// **The fast walk against the plain one, over every short string a hostile alphabet builds.**
    ///
    /// `the_fast_walk_and_the_plain_one_agree` uses fixtures chosen by hand to attack the argument,
    /// which is only ever as good as the imagination behind them. This enumerates instead: all
    /// three- and four-character strings over 22 characters picked to break grapheme segmentation —
    /// `NUL`, `CR`, `LF`, `TAB`, `DEL`, `ESC`, a combining mark, a `Prepend`, ZWSP, ZWJ, VS16, a
    /// wide CJK character, a regional indicator, an emoji and a skin-tone modifier, Devanagari
    /// spacing and virama marks, and a fullwidth Latin letter. **244,904 strings against both cell
    /// models**, in about 1.5 s.
    ///
    /// Also written by an adversarial reviewer as a throwaway and kept: it is a stronger check than
    /// the hand-picked one and costs almost nothing to run.
    #[test]
    fn the_fast_walk_agrees_on_every_short_hostile_combination() {
        let alphabet: Vec<char> = vec![
            '\u{0}',
            '\r',
            '\n',
            '\t',
            'a',
            ' ',
            '\u{7f}',
            '\u{1b}',
            '\u{e9}',
            '\u{301}',
            '\u{605}',
            '\u{200b}',
            '\u{200d}',
            '\u{fe0f}',
            '日',
            '\u{1F1EC}',
            '\u{1F44D}',
            '\u{1F3FB}',
            '\u{903}',
            '\u{94d}',
            'क',
            '\u{ff21}',
        ];
        let models = [CellModel::new(), CellModel::revealing()];
        let mut failures: Vec<String> = Vec::new();
        let n = alphabet.len();

        let check = |s: &str, failures: &mut Vec<String>| {
            for m in &models {
                let plain: Vec<Cell> = m.cells(s).collect();
                let fast: Vec<Cell> = m.walk(s).collect();
                if plain != fast && failures.len() < 40 {
                    failures.push(format!(
                        "{:?} reveal={} plain={:?} fast={:?}",
                        s, m.reveal_invisibles, plain, fast
                    ));
                }
            }
        };

        for i in 0..n {
            for j in 0..n {
                for k in 0..n {
                    let mut s = String::new();
                    s.push(alphabet[i]);
                    s.push(alphabet[j]);
                    s.push(alphabet[k]);
                    check(&s, &mut failures);
                }
            }
        }
        for i in 0..n {
            for j in 0..n {
                for k in 0..n {
                    for l in 0..n {
                        let mut s = String::new();
                        s.push(alphabet[i]);
                        s.push(alphabet[j]);
                        s.push(alphabet[k]);
                        s.push(alphabet[l]);
                        check(&s, &mut failures);
                    }
                }
            }
        }
        for f in &failures {
            println!("DIFF {f}");
        }
        assert!(failures.is_empty(), "{} divergences", failures.len());
    }

    #[test]
    fn ascii_is_one_cell_each() {
        assert_eq!(width("hello"), 5);
        assert_eq!(width(""), 0);
    }

    /// §3.3: East Asian Wide and Fullwidth clusters occupy two cells. This is what makes a column of
    /// CJK log messages line up with a column of ASCII ones.
    #[test]
    fn east_asian_wide_and_fullwidth_are_two_cells() {
        assert_eq!(width("日本語"), 6, "CJK ideographs are wide");
        assert_eq!(width("ｆｕｌｌ"), 8, "fullwidth Latin is wide");
        assert_eq!(width("한국어"), 6, "Hangul syllables are wide");
        assert_eq!(width("ｱｲｳ"), 3, "halfwidth katakana is narrow");
    }

    /// §3.3: combining marks occupy zero additional cells — true of **non-spacing** marks, which is
    /// what that bullet means. See the spacing-mark test below for the half §3.3 does not say.
    #[test]
    fn non_spacing_combining_marks_add_nothing() {
        assert_eq!(width("e\u{301}"), 1, "e + combining acute is one cell");
        assert_eq!(width("é"), 1, "and so is the precomposed form");
        assert_eq!(
            width("a\u{300}\u{301}\u{302}"),
            1,
            "a base with three marks is still one cluster"
        );
    }

    /// **§3.3's "combining marks occupy 0 additional cells" is imprecise, and this is the case it
    /// gets wrong.** A Devanagari matra is a *spacing* mark (`Mc`): part of the same grapheme
    /// cluster, but carrying its own advance, so `कि` occupies two cells and not one.
    ///
    /// An earlier version of this file asserted one. Devanagari is named in §3.3's own acceptance
    /// test, so that fixture would have rendered with every following column a cell out — while a
    /// test claimed it was right.
    #[test]
    fn spacing_marks_do_take_a_cell_even_though_they_combine() {
        let ki = "\u{915}\u{93F}";
        assert_eq!(
            CellModel::new().cells(ki).count(),
            1,
            "ka + vowel sign i is one cluster and must not segment into two"
        );
        assert_eq!(width(ki), 2, "but it is two cells wide");
        assert_eq!(width("\u{915}\u{94D}\u{937}\u{93F}"), 3, "क्षि");
        assert_eq!(width("\u{995}\u{9BF}"), 2, "Bengali");
        assert_eq!(width("\u{BA8}\u{BBF}"), 2, "Tamil");
        assert_eq!(width("a\u{E33}"), 2, "Thai SARA AM");
    }

    /// Conjoining jamo: each leading jamo in a run carries its own advance, so a cluster is wider
    /// than its base. This is the counterexample that killed the "width from the base" rule.
    #[test]
    fn multi_jamo_hangul_clusters_are_wider_than_their_base() {
        let two_l = "\u{1100}\u{1100}\u{1161}";
        assert_eq!(
            CellModel::new().cells(two_l).count(),
            1,
            "conjoining jamo form one cluster"
        );
        assert_eq!(width(two_l), 4, "two leading jamo plus a vowel");
        assert_eq!(width("\u{1100}\u{1100}\u{1100}\u{1161}"), 6);
        assert_eq!(
            width("\u{1100}\u{1161}\u{11A8}"),
            2,
            "the ordinary L V T case, where V and T are zero-width"
        );
    }

    /// **A stray variation selector is attacker-controllable** — §13.4 notes log content is
    /// frequently attacker-influenced — and must not move a line's columns. VS16 and VS15 mean
    /// something only on a base with `Emoji=Yes`; anywhere else they are defective sequences.
    ///
    /// An earlier version treated *any* cluster containing VS16 as two cells and any containing
    /// VS15 as one, so appending three bytes anywhere in a line shifted every column to its right.
    /// §13.4 calls a viewer that can be made to lie a real defect, and that was one.
    #[test]
    fn a_stray_variation_selector_does_not_move_the_columns() {
        assert_eq!(
            width("A\u{FE0F}"),
            1,
            "VS16 on a non-emoji base does nothing"
        );
        assert_eq!(width(" \u{FE0F}"), 1, "nor on a space");
        assert_eq!(width("\u{E01}\u{FE0F}"), 1, "nor on Thai");
        assert_eq!(width("\u{FE0F}"), 0, "and alone it draws nothing at all");

        assert_eq!(
            width("😀\u{FE0E}"),
            2,
            "VS15 does not narrow an emoji to one cell"
        );
        assert_eq!(width("日\u{FE0E}"), 2, "nor a wide ideograph");
        assert_eq!(width("\u{FE0E}"), 0);
    }

    /// **The error this module exists to prevent.** A ZWJ family is four two-cell code points; a
    /// width that sums across the cluster says 8, and every column after it is six cells out.
    #[test]
    fn a_zwj_emoji_sequence_is_one_cluster_of_two_cells() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
        assert!(family.contains(ZWJ), "the fixture must actually be joined");
        assert_eq!(
            CellModel::new().cells(family).count(),
            1,
            "a ZWJ sequence is one grapheme cluster"
        );
        assert_eq!(width(family), 2, "and it draws in two cells, not eight");
    }

    #[test]
    fn emoji_and_skin_tone_modifiers_are_two_cells() {
        assert_eq!(width("😀"), 2);
        assert_eq!(width("\u{1F44D}\u{1F3FD}"), 2, "thumbs up + skin tone");
        assert_eq!(width("🇬🇧"), 2, "a regional-indicator pair is one flag");
        assert_eq!(width("🇬🇧🇯🇵"), 4, "two flags");
    }

    /// The selectors exist to override the default presentation, so they win.
    #[test]
    fn presentation_selectors_decide_the_width() {
        assert_eq!(
            width("\u{2714}\u{FE0F}"),
            2,
            "check mark forced to emoji presentation"
        );
        assert_eq!(
            width("\u{2714}\u{FE0E}"),
            1,
            "check mark forced to text presentation"
        );
    }

    #[test]
    fn box_drawing_is_one_cell() {
        assert_eq!(width("┌─┬─┐"), 5, "box drawing must not be treated as wide");
        assert_eq!(width("│ │"), 3);
    }

    /// RTL text is laid out right-to-left by the shaper, but its *cell count* is unaffected —
    /// direction is a painting concern, not an arithmetic one.
    #[test]
    fn arabic_counts_cells_the_same_as_any_other_script() {
        assert_eq!(width("سلام"), 4);
    }

    /// §5.6: control bytes and NULs render as a visible replacement glyph and are never silently
    /// dropped. A zero-cell control would be silently dropped as far as the grid is concerned.
    #[test]
    fn control_characters_take_a_cell_so_they_cannot_vanish() {
        assert_eq!(width("a\u{0}b"), 3, "a NUL is visible");
        assert_eq!(width("a\u{7}b"), 3, "and so is a bell");
        assert_eq!(width("a\u{1B}b"), 3, "and a stray escape");
    }

    /// §13.4's reveal-invisibles toggle is a width question, not only a painting one.
    #[test]
    fn zero_width_characters_are_zero_until_revealed() {
        let zwsp = "a\u{200B}b";
        let bidi_override = "a\u{202E}b";
        assert_eq!(
            width(zwsp),
            2,
            "a zero-width space draws nothing by default"
        );
        assert_eq!(width(bidi_override), 2, "nor does a bidi override");
        assert_eq!(
            CellModel::revealing().cell_count(zwsp),
            3,
            "revealed, it takes a cell"
        );
        assert_eq!(
            CellModel::revealing().cell_count(bidi_override),
            3,
            "which is how a Trojan Source override becomes visible (§13.4)"
        );
    }

    #[test]
    fn cells_report_their_byte_and_column_positions() {
        let model = CellModel::new();
        let placed: Vec<Cell> = model.cells("a日b").collect();
        assert_eq!(
            placed,
            vec![
                Cell {
                    byte: 0,
                    byte_len: 1,
                    cell: 0,
                    width: 1
                },
                Cell {
                    byte: 1,
                    byte_len: 3,
                    cell: 1,
                    width: 2
                },
                Cell {
                    byte: 4,
                    byte_len: 1,
                    cell: 3,
                    width: 1
                },
            ]
        );
    }

    /// Hit-testing. A click on either half of a wide cluster selects the whole cluster — a byte
    /// offset inside it is not something the decoder or the user can honour.
    #[test]
    fn a_click_on_either_half_of_a_wide_cluster_lands_on_the_cluster() {
        let model = CellModel::new();
        let line = "a日b";
        assert_eq!(model.byte_at_cell(line, 0), 0, "the ascii a");
        assert_eq!(model.byte_at_cell(line, 1), 1, "left half of 日");
        assert_eq!(
            model.byte_at_cell(line, 2),
            1,
            "right half of 日, same cluster"
        );
        assert_eq!(model.byte_at_cell(line, 3), 4, "the ascii b");
        assert_eq!(
            model.byte_at_cell(line, 99),
            line.len(),
            "past the end clamps"
        );
    }

    /// **A zero-width cluster occupies no column, so it cannot be clicked.** Giving it a phantom
    /// one — which an earlier version did, via `width.max(1)` — means clicking the first visible
    /// character returns the byte offset of an invisible one *before* it, and a caret placed from
    /// that click sits where the user did not click.
    ///
    /// `"\u{202E}abc"` is the Trojan Source line §13.4 names, and it is exactly this shape: the
    /// override is cluster 0 at column 0, and so is `a`.
    #[test]
    fn an_invisible_cluster_does_not_steal_the_next_ones_column() {
        let model = CellModel::new();

        let trojan = "\u{202E}abc";
        assert_eq!(
            model.byte_at_cell(trojan, 0),
            3,
            "column 0 paints `a`, not the override that precedes it"
        );

        let zwsp = "a\u{200B}b";
        assert_eq!(model.byte_at_cell(zwsp, 0), 0, "the a");
        assert_eq!(model.byte_at_cell(zwsp, 1), 4, "column 1 paints the b");

        let wide = "\u{65E5}\u{200B}b";
        assert_eq!(
            model.byte_at_cell(wide, 2),
            6,
            "past a wide cluster and a ZWSP"
        );

        let trailing = "a\u{200B}";
        assert_eq!(
            model.byte_at_cell(trailing, 1),
            trailing.len(),
            "a trailing invisible clamps to end of line, so a selection keeps it"
        );
    }

    /// The half of §13.4 the toggle does **not** yet do. Asserted so the gap is visible rather than
    /// discovered later — see [`CellModel`] for why it needs `General_Category` data to fix.
    #[test]
    fn revealing_does_not_yet_reach_an_invisible_inside_a_cluster() {
        let revealing = CellModel::revealing();

        assert_eq!(
            revealing.cell_count("a\u{200B}"),
            2,
            "its own cluster: revealed"
        );
        assert_eq!(
            revealing.cell_count("\u{202E}abc"),
            4,
            "and so is a bidi override, which is the Trojan Source case"
        );

        for hidden in ["a\u{200D}", "a\u{E0067}", "a\u{FE00}"] {
            assert_eq!(
                revealing.cell_count(hidden),
                1,
                "{hidden:?} is absorbed into the `a` cluster and stays hidden — §13.4 is not met \
                 for this shape yet"
            );
        }
    }

    #[test]
    fn byte_and_cell_positions_round_trip() {
        let model = CellModel::new();
        for line in [
            "hello",
            "a日b",
            "e\u{301}x",
            "🇬🇧 flag",
            "日本語のログ",
            "\u{202E}abc",
            "a\u{200B}b",
            "\u{915}\u{93F}x",
        ] {
            for cluster in model.cells(line) {
                assert_eq!(
                    model.cell_at_byte(line, cluster.byte),
                    cluster.cell,
                    "byte {} of {line:?}",
                    cluster.byte
                );
                if cluster.width > 0 {
                    assert_eq!(
                        model.byte_at_cell(line, cluster.cell),
                        cluster.byte,
                        "cell {} of {line:?}",
                        cluster.cell
                    );
                }
                // A byte inside a multi-byte cluster resolves to that cluster, not the next.
                for inside in cluster.byte..cluster.byte + cluster.byte_len {
                    assert_eq!(
                        model.cell_at_byte(line, inside),
                        cluster.cell,
                        "byte {inside} is inside the cluster at {}",
                        cluster.byte
                    );
                }
            }
        }
    }

    /// §3.3's horizontal-extent rule leans on this: no encoding produces more cells than bytes, so
    /// a line's byte length is a safe upper bound for the scrollbar before layout has run.
    #[test]
    fn cell_count_never_exceeds_byte_length() {
        for line in [
            "hello",
            "日本語のログ",
            "e\u{301}\u{302}",
            "🇬🇧🇯🇵",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "ｆｕｌｌｗｉｄｔｈ",
            "┌─┬─┐",
            "a\u{0}b\u{1B}c",
        ] {
            assert!(
                width(line) <= line.len(),
                "{line:?}: {} cells against {} bytes",
                width(line),
                line.len()
            );
        }
    }

    /// **The early exit must not change a single answer.** `byte_span` stops once the clusters are
    /// past the requested range, which turned a per-row full-line walk into a bounded one — but it
    /// is a change to the module whose whole history is that every hand-written rule about text was
    /// wrong in at least four ways. So this runs the loop with the exit removed and requires the
    /// two to agree on every range of every fixture, including the zero-width boundary cases the
    /// outward rounding exists for.
    /// **An anchor must never change an answer, only how fast it is reached.**
    ///
    /// The whole safety argument for `ColumnAnchors` is that it is a hint: every lookup is exact
    /// whatever anchors it is given, including none. That is asserted here rather than argued, by
    /// running the anchored and unanchored paths against each other over **every** byte offset,
    /// every column and every column *range* of fixtures chosen to be hostile — zero-width clusters
    /// that make several anchors share a column, wide clusters, a ZWJ sequence, and regional
    /// indicators, whose UAX #29 pairing is the one rule that could plausibly break on resume.
    ///
    /// Strides of 1, 2 and 3 are used, not 64: on a short fixture 64 produces a single anchor at the
    /// origin and tests nothing, while stride 1 makes every cluster a resume point.
    #[test]
    fn an_anchor_never_changes_an_answer() {
        let lines = [
            "hello world",
            "日本語のログ",
            "a\u{0301}e\u{0301}i\u{0301}o\u{0301}",
            "\u{202E}abc\u{202C}def",
            "x\u{200B}y\u{200B}z",
            "👨‍👩‍👧‍👦 family",
            "🇬🇧🇫🇷🇩🇪 flags",
            "mixed 日本 a\u{0301} \u{200B} 👍🏻 end",
            "café — naïve — résumé",
            &"日".repeat(50),
        ];

        for model in [CellModel::new(), CellModel::revealing()] {
            for line in lines {
                let plain = ColumnAnchors::none();
                for stride in 1..=3 {
                    let anchors = ColumnAnchors::build_with_stride(&model, line, stride);
                    assert!(!anchors.is_empty(), "{line:?} produced no anchors");

                    for byte in 0..=line.len() + 1 {
                        assert_eq!(
                            model.cell_at_byte_anchored(line, byte, &anchors),
                            model.cell_at_byte_anchored(line, byte, &plain),
                            "cell_at_byte({line:?}, {byte}) stride {stride}"
                        );
                    }
                    let cells = model.cell_count(line);
                    for cell in 0..=cells + 2 {
                        assert_eq!(
                            model.byte_at_cell_anchored(line, cell, &anchors),
                            model.byte_at_cell_anchored(line, cell, &plain),
                            "byte_at_cell({line:?}, {cell}) stride {stride}"
                        );
                    }
                    for start in 0..=cells + 1 {
                        for end in 0..=cells + 1 {
                            assert_eq!(
                                model.byte_span_anchored(line, start..end, &anchors),
                                model.byte_span_anchored(line, start..end, &plain),
                                "byte_span({line:?}, {start}..{end}) stride {stride}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// The real stride, on a line the size §10.3 supports, actually samples — and the lookup it
    /// enables lands on the same bytes as the walk it replaces.
    #[test]
    fn a_long_line_is_anchored_and_still_exact() {
        let model = CellModel::new();
        // 19.4 KB with a leading non-ASCII character: the shape measured at 76 ms a frame.
        let line = format!("— {}", "field=value;".repeat(1600));
        assert!(!CellModel::is_column_per_byte(&line));

        let anchors = ColumnAnchors::build(&model, &line);
        assert!(
            anchors.marks.len() > 100,
            "only {} anchors for a {}-byte line",
            anchors.marks.len(),
            line.len()
        );

        let plain = ColumnAnchors::none();
        let cells = model.cell_count(&line);
        for cell in [
            0,
            1,
            63,
            64,
            65,
            1000,
            cells / 2,
            cells - 1,
            cells,
            cells + 5,
        ] {
            assert_eq!(
                model.byte_at_cell_anchored(&line, cell, &anchors),
                model.byte_at_cell_anchored(&line, cell, &plain),
                "byte_at_cell at {cell}"
            );
            let span = 0..80;
            assert_eq!(
                model.byte_span_anchored(&line, cell..cell + span.end, &anchors),
                model.byte_span_anchored(&line, cell..cell + span.end, &plain),
                "byte_span at {cell}"
            );
        }
    }

    /// **The fast walk must produce the identical sequence, cluster for cluster.**
    ///
    /// `CellModel::walk` skips `grapheme_indices` and `unicode-width` for ASCII bytes on a local
    /// argument about UAX #29. The argument is plausible and this module's history is that plausible
    /// arguments about text are wrong in several ways at once, so it is checked against `cells` —
    /// the canonical walk — over fixtures chosen to attack the argument's edges: the `CR LF` pair
    /// GB3 exempts, a `Prepend` character before ASCII (GB9b), combining marks that absorb the ASCII
    /// *before* them, ASCII directly abutting wide and zero-width clusters, and regional indicators
    /// whose pairing depends on where segmentation resumed.
    #[test]
    fn the_fast_walk_and_the_plain_one_agree() {
        let lines: Vec<String> = vec![
            String::new(),
            "a".into(),
            "hello world".into(),
            "2026-08-11T12:00:00Z INFO Worker[3] batch 41 in 9ms".into(),
            // GB3: the one ASCII pair that does not break.
            "a\r\nb".into(),
            "\r\n".into(),
            "a\r".into(),
            "\ra".into(),
            // GB9b: U+0605 ARABIC NUMBER MARK ABOVE is Prepend, and absorbs the ASCII after it.
            "x\u{0605}9y".into(),
            "\u{0605}a".into(),
            // A combining mark absorbing the ASCII before it.
            "abc\u{0301}def".into(),
            "a\u{0301}".into(),
            // ASCII abutting wide, zero-width and emoji clusters.
            "ab日本cd".into(),
            "ab\u{200B}cd".into(),
            "ab\u{202E}cd".into(),
            "ab👨‍👩‍👧‍👦cd".into(),
            "ab👍🏻cd".into(),
            "ab🇬🇧🇫🇷cd".into(),
            // Runs long enough to exercise the scan rather than only its edges.
            format!("— {}", "field=value;".repeat(200)),
            format!("{}—{}", "x".repeat(300), "y".repeat(300)),
            "日".repeat(200),
            format!("a\u{0301}{}", "z".repeat(500)),
        ];

        // **The two fixtures the argument turns on must actually be doing work.** Both controls on
        // `ascii_singleton` happened to fail first on `"a\r\nb"`, which would leave GB9b untested
        // while looking covered. These assert the awkward clusters exist before anything is compared.
        let m = CellModel::new();
        assert_eq!(
            m.cells("a\r\nb").count(),
            3,
            "CR LF is not being treated as one cluster, so GB3 is untested here"
        );
        assert_eq!(
            m.cells("x\u{0605}9y").count(),
            3,
            "U+0605 is not absorbing the ASCII after it, so GB9b is untested here"
        );

        for model in [CellModel::new(), CellModel::revealing()] {
            for line in &lines {
                let plain: Vec<Cell> = model.cells(line).collect();
                let fast: Vec<Cell> = model.walk(line).collect();
                assert_eq!(
                    fast, plain,
                    "the fast walk disagrees on {line:?} (reveal={})",
                    model.reveal_invisibles
                );
            }
        }
    }

    /// **Anchors from the wrong line are ignored, not trusted, and never panic.**
    ///
    /// An adversarial review of the anchor change reproduced a real panic here —
    /// `start byte index 384 is out of bounds for string of length 18` — by handing
    /// `byte_span_anchored` a set built from a longer line, and then correctly refuted it as a live
    /// bug: `Rows` builds `anchors[i]` from `lines[i]` and `paint.rs` pairs them in a single loop
    /// iteration, so no caller can mismatch them. It is fixed anyway, because the module promises in
    /// writing that a stale anchor set costs speed and not correctness, and a panic out of a public
    /// function is not a slower answer.
    ///
    /// The pairing that *is* dangerous is subtler and is the reason this asserts equality rather
    /// than merely absence of a panic: anchors from a **same-length** line whose clusters fall
    /// differently land on valid boundaries and would silently resume at a wrong column, which for
    /// `byte_span` is §5.6 content loss rather than a crash.
    #[test]
    fn anchors_from_another_line_are_ignored_rather_than_believed() {
        let model = CellModel::new();
        let long = "日本語のログ ".repeat(40);
        let foreign = ColumnAnchors::build_with_stride(&model, &long, 2);
        assert!(!foreign.is_empty());

        // Shorter than the anchors' byte offsets, and of a different cluster shape.
        for line in ["short", "", "日本", "a\u{0301}bc", "\u{202E}abc"] {
            let plain = ColumnAnchors::none();
            let cells = model.cell_count(line);
            for cell in 0..=cells + 2 {
                assert_eq!(
                    model.byte_at_cell_anchored(line, cell, &foreign),
                    model.byte_at_cell_anchored(line, cell, &plain),
                    "byte_at_cell({line:?}, {cell}) with foreign anchors"
                );
            }
            for start in 0..=cells + 1 {
                for end in 0..=cells + 1 {
                    assert_eq!(
                        model.byte_span_anchored(line, start..end, &foreign),
                        model.byte_span_anchored(line, start..end, &plain),
                        "byte_span({line:?}, {start}..{end}) with foreign anchors"
                    );
                }
            }
            for byte in 0..=line.len() + 1 {
                assert_eq!(
                    model.cell_at_byte_anchored(line, byte, &foreign),
                    model.cell_at_byte_anchored(line, byte, &plain),
                    "cell_at_byte({line:?}, {byte}) with foreign anchors"
                );
            }
        }
    }

    /// A short or all-ASCII line is not worth anchoring, and `build` says so.
    #[test]
    fn lines_that_gain_nothing_are_not_anchored() {
        let model = CellModel::new();
        assert!(ColumnAnchors::build(&model, &"x".repeat(5000)).is_empty());
        assert!(ColumnAnchors::build(&model, "日本").is_empty());
        assert!(!ColumnAnchors::build(&model, &"日".repeat(500)).is_empty());
    }

    /// The ASCII fast path must be an **identity**, not an approximation.
    ///
    /// `is_column_per_byte` skips the cluster walk entirely on the strength of an argument about
    /// ASCII, and an argument is not evidence. This runs both paths over every offset and every
    /// range of a set of ASCII fixtures — including the control characters and the lone `\r` that
    /// the argument turns on — and requires them to agree exactly. It runs under **both** cell
    /// models, because the claim includes "independent of `reveal_invisibles`".
    #[test]
    fn a_line_of_ascii_agrees_with_the_full_walk() {
        // The walk, forced, by reusing the general path through a line the fast path rejects: a
        // sentinel non-ASCII character is appended and the results are only compared over the
        // prefix, so the oracle is the module's own code rather than a copy of it.
        fn walked(m: &CellModel, line: &str) -> Vec<Cell> {
            m.cells(line).collect()
        }

        let lines = [
            "",
            "a",
            "hello world",
            "2026-08-11T12:00:00Z INFO  Worker[3] processed batch 41 in 9ms",
            "tabs\tand\tspaces",
            "a\u{7}b\u{1b}c\u{0}d",
            "trailing\r",
            "\u{1}\u{2}\u{3}",
            &"x".repeat(300),
        ];

        for model in [CellModel::new(), CellModel::revealing()] {
            for line in lines {
                assert!(
                    CellModel::is_column_per_byte(line),
                    "fixture {line:?} is not on the fast path, so it proves nothing"
                );

                // Every ASCII byte is one cluster of one cell — the whole basis of the shortcut.
                let clusters = walked(&model, line);
                assert_eq!(
                    clusters.len(),
                    line.len(),
                    "{line:?} is not one cluster a byte"
                );
                for (i, c) in clusters.iter().enumerate() {
                    assert_eq!(
                        (c.byte, c.byte_len, c.cell, c.width),
                        (i, 1, i, 1),
                        "{line:?}"
                    );
                }

                assert_eq!(model.cell_count(line), line.len(), "cell_count {line:?}");

                for byte in 0..=line.len() + 2 {
                    assert_eq!(
                        model.cell_at_byte(line, byte),
                        byte.min(line.len()),
                        "cell_at_byte({line:?}, {byte})"
                    );
                }
                for cell in 0..=line.len() + 2 {
                    assert_eq!(
                        model.byte_at_cell(line, cell),
                        cell.min(line.len()),
                        "byte_at_cell({line:?}, {cell})"
                    );
                }
                for start in 0..=line.len() + 1 {
                    for end in 0..=line.len() + 1 {
                        let got = model.byte_span(line, start..end);
                        let want = if start >= end {
                            let at = start.min(line.len());
                            at..at
                        } else {
                            start.min(line.len())..end.min(line.len())
                        };
                        assert_eq!(got, want, "byte_span({line:?}, {start}..{end})");
                    }
                }
            }
        }

        // And the guard actually excludes what it claims to: anything non-ASCII, and any `\n`.
        for line in ["café", "日本", "a\u{0301}", "a\nb", "\r\n", "👍🏻"] {
            assert!(
                !CellModel::is_column_per_byte(line),
                "{line:?} took the fast path and its columns are not its bytes"
            );
        }
    }

    #[test]
    fn the_early_exit_agrees_with_a_full_scan_on_every_range() {
        /// `byte_span` with the early exit deleted — the previous implementation, kept here as the
        /// oracle rather than in the module.
        fn full_scan(
            m: &CellModel,
            line: &str,
            cells: core::ops::Range<usize>,
        ) -> core::ops::Range<usize> {
            if cells.start >= cells.end {
                let at = m.byte_at_cell(line, cells.start);
                return at..at;
            }
            let (mut start, mut end) = (None, None);
            for cluster in m.cells(line) {
                let starts_here = cluster.cell + cluster.width > cells.start
                    || (cluster.width == 0 && cluster.cell >= cells.start);
                if start.is_none() && starts_here {
                    start = Some(cluster.byte);
                }
                let inside =
                    cluster.cell < cells.end || (cluster.width == 0 && cluster.cell == cells.end);
                if start.is_some() && inside {
                    end = Some(cluster.byte + cluster.byte_len);
                }
            }
            let start = start.unwrap_or(line.len());
            start..end.unwrap_or(start).max(start)
        }

        let lines = [
            "",
            "plain ascii log line",
            "\u{202E}abc",
            "abc\u{202E}",
            "a\u{200B}\u{FE0F}b",
            "日本語のログ行",
            "a日b語c",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467} family",
            "कि and ि alone",
            "ｆｕｌｌｗｉｄｔｈ",
            "a\u{0}b\u{1B}c",
            "\u{202E}\u{200B}\u{200D}",
        ];
        for model in [CellModel::new(), CellModel::revealing()] {
            for line in lines {
                let cells = model.cell_count(line);
                for start in 0..=cells + 2 {
                    for end in 0..=cells + 2 {
                        let range = start..end;
                        assert_eq!(
                            model.byte_span(line, range.clone()),
                            full_scan(&model, line, range.clone()),
                            "{line:?} over {range:?}, reveal={}",
                            model.reveal_invisibles
                        );
                    }
                }
            }
        }
    }
}
