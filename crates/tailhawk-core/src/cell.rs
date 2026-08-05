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

    /// Total cells the line occupies — its horizontal extent.
    pub fn cell_count(&self, line: &str) -> usize {
        self.cells(line).map(|c| c.width).sum()
    }

    /// The cluster containing a byte offset, for turning a byte position into a column.
    ///
    /// A byte in the middle of a cluster resolves to that cluster, not to the next one.
    pub fn cell_at_byte(&self, line: &str, byte: usize) -> usize {
        let mut last = 0;
        for cluster in self.cells(line) {
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
        for cluster in self.cells(line) {
            if cluster.width > 0 && cell < cluster.cell + cluster.width {
                return cluster.byte;
            }
        }
        line.len()
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
}
