//! Visual reordering — UAX #9 rule L2, and nothing else.
//!
//! **This is the piece `shape.rs` measured itself into owing.** `GetGlyphs` returns glyphs in
//! *logical* order even for a run it was told is right-to-left — the opposite of the obvious guess,
//! and `directwrite_returns_glyphs_in_logical_order` pins it. So something has to put them in the
//! order they are painted, and that something is here rather than in `shape.rs`: it is pure
//! arithmetic over resolved bidi levels, it holds no device, and it is testable against the standard
//! without a font.
//!
//! ## Rule L2, and why the loop counts *down*
//!
//! > From the highest level found in the text to the lowest odd level, reverse any contiguous
//! > sequence of characters that are at that level or higher.
//!
//! The descending order is the whole algorithm. Each pass reverses maximal windows at its level and
//! above, so an inner embedding is reversed first and then carried *as a block* by the reversal of
//! the run containing it — which is exactly what nesting means. Running the levels the other way
//! reverses the outer run first and then flips the inner one back inside it, giving a visually
//! plausible line with the embedded fragment backwards.
//!
//! **"Lowest odd level" is not "lowest level".** An all-even line has no odd level at all and must
//! not be reordered; taking the minimum level instead would start the loop at 0, whose window is the
//! whole line, and **reverse every left-to-right line in the file**. The sentinel is a level above
//! anything representable, so the range is empty when no odd level exists and the identity falls out
//! rather than being special-cased.
//!
//! ## Levels are per *item*, and the caller chooses what an item is
//!
//! Feed it one level per run and it orders runs; feed it one per glyph and it orders glyphs. The
//! algorithm does not care, and neither granularity is privileged — a painter walking runs wants the
//! first, a painter emitting a flat glyph list wants the second. [`Shaped::visual_glyphs`] uses it
//! both ways.
//!
//! [`Shaped::visual_glyphs`]: crate::shape::Shaped::visual_glyphs

/// The order to paint items in, left to right, given each item's resolved bidi level.
///
/// Returns indices into `levels`. Always a permutation of `0..levels.len()`, and the identity for
/// text with no right-to-left content — which is the overwhelmingly common case in a log file and
/// costs one pass to establish.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    reorder(levels, &mut order);
    order
}

/// [`visual_order`] in place, for a caller that already has the buffer.
///
/// `order` must be `0..levels.len()`; anything else is permuted rather than validated, which is the
/// caller's problem and not worth a `Result` on a function this hot.
pub fn reorder(levels: &[u8], order: &mut [usize]) {
    let Some(&highest) = levels.iter().max() else {
        return;
    };
    // A level above `DWRITE_..`'s and UAX #9's maximum depth of 125, so "no odd level anywhere"
    // leaves `lowest_odd > highest` and the range below is empty.
    let lowest_odd = levels
        .iter()
        .copied()
        .filter(|level| level % 2 == 1)
        .min()
        .unwrap_or(u8::MAX);
    if lowest_odd > highest {
        return;
    }

    for level in (lowest_odd..=highest).rev() {
        let mut at = 0;
        while at < levels.len() {
            if levels[at] < level {
                at += 1;
                continue;
            }
            let mut end = at;
            while end < levels.len() && levels[end] >= level {
                end += 1;
            }
            order[at..end].reverse();
            at = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `visual_order` on one level per character, for readability in the tests below.
    fn visual(text: &str, levels: &[u8]) -> String {
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(chars.len(), levels.len(), "one level per character");
        visual_order(levels).iter().map(|&i| chars[i]).collect()
    }

    #[test]
    fn a_line_with_no_right_to_left_content_is_not_reordered() {
        assert_eq!(visual_order(&[0, 0, 0, 0]), vec![0, 1, 2, 3]);
        assert_eq!(visual_order(&[2, 2, 0, 2]), vec![0, 1, 2, 3]);
    }

    #[test]
    fn an_empty_line_orders_to_nothing() {
        assert_eq!(visual_order(&[]), Vec::<usize>::new());
    }

    #[test]
    fn a_single_item_is_its_own_order_at_any_level() {
        for level in 0..=8 {
            assert_eq!(visual_order(&[level]), vec![0]);
        }
    }

    /// UAX #9's own worked example, with capitals standing in for right-to-left characters.
    #[test]
    fn car_means_car_reverses_only_the_right_to_left_span() {
        let text = "car means CAR.";
        let mut levels = vec![0u8; text.len()];
        levels[10..13].fill(1);
        assert_eq!(visual(text, &levels), "car means RAC.");
    }

    /// A right-to-left paragraph containing a left-to-right phrase: the phrase keeps its own order
    /// while the line around it runs the other way.
    #[test]
    fn an_embedded_left_to_right_phrase_survives_the_reversal_of_the_run_around_it() {
        //  A B  are RTL, "ok" is an LTR embedding inside them, C D are RTL again.
        let text = "ABokCD";
        let levels = [1, 1, 2, 2, 1, 1];
        assert_eq!(visual(text, &levels), "DCokBA");
    }

    /// The pass order is the algorithm. Ascending reverses the outer run first and then flips the
    /// embedded phrase back inside it.
    ///
    /// **The nesting has to be off-centre for this to show anything**, and the first version of this
    /// test was not. Reversing a window and then reversing a sub-window *symmetric about its centre*
    /// gives the same answer either way round — `ABokCD` at levels `[1,1,2,2,1,1]` agrees under both
    /// orders, and reads as a passing test of a property it cannot see. `de` sitting at 3..5 of a
    /// six-item run does not have that symmetry.
    #[test]
    fn the_embedded_phrase_is_backwards_if_the_levels_are_walked_the_other_way() {
        let text = "ABCdeF";
        let levels = [1u8, 1, 1, 2, 2, 1];
        assert_eq!(visual(text, &levels), "FdeCBA");
        let ascending = {
            let mut order: Vec<usize> = (0..levels.len()).collect();
            for level in 1..=2u8 {
                let mut at = 0;
                while at < levels.len() {
                    if levels[at] < level {
                        at += 1;
                        continue;
                    }
                    let mut end = at;
                    while end < levels.len() && levels[end] >= level {
                        end += 1;
                    }
                    order[at..end].reverse();
                    at = end;
                }
            }
            order
        };
        let chars: Vec<char> = text.chars().collect();
        let wrong: String = ascending.iter().map(|&i| chars[i]).collect();
        assert_eq!(wrong, "FedBCA", "the embedded phrase comes out reversed");
        assert_ne!(wrong, visual(text, &levels));
    }

    #[test]
    fn a_uniformly_right_to_left_line_is_reversed_whole() {
        assert_eq!(visual_order(&[1, 1, 1, 1]), vec![3, 2, 1, 0]);
        assert_eq!(visual_order(&[3, 3, 3]), vec![2, 1, 0]);
    }

    /// Three levels deep, which is where a single reversal pass stops being enough.
    ///
    /// Off-centre for the same reason as the test above — `[0,1,2,2,1,0]` reads as a fine fixture and
    /// is symmetric, so the ascending control does not fire on it.
    #[test]
    fn a_doubly_nested_embedding_carries_its_inner_block_as_a_unit() {
        //          a    B  C  d  e  F
        let levels = [0u8, 1, 1, 2, 2, 1];
        assert_eq!(visual("aBCdeF", &levels), "aFdeCB");
    }

    /// A run at level 3 with nothing at 1 or 2 around it reverses like any other odd run — the loop
    /// walks the levels between, and their windows are empty.
    #[test]
    fn a_deeply_embedded_run_with_nothing_at_the_levels_below_it_still_reverses() {
        let text = "abCDef";
        let levels = [0u8, 0, 3, 3, 0, 0];
        assert_eq!(visual(text, &levels), "abDCef");
    }

    #[test]
    fn the_result_is_always_a_permutation() {
        let cases: [&[u8]; 7] = [
            &[],
            &[0],
            &[1, 0, 1],
            &[0, 1, 2, 1, 0],
            &[2, 2, 1, 1, 2, 2],
            &[5, 4, 3, 2, 1, 0],
            &[1, 1, 1, 2, 2, 3, 3, 2, 1],
        ];
        for levels in cases {
            let order = visual_order(levels);
            let mut sorted = order.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..levels.len()).collect::<Vec<_>>(),
                "levels {levels:?} gave {order:?}"
            );
        }
    }

    /// An exhaustive sweep of every level assignment up to four items and three levels. It exists
    /// because the invariants below are cheap to state and the cases that break them are not ones a
    /// hand-picked fixture finds.
    #[test]
    fn every_short_level_assignment_holds_the_invariants() {
        for len in 0..=4usize {
            let mut levels = vec![0u8; len];
            for encoded in 0..4u32.pow(len as u32) {
                for (i, level) in levels.iter_mut().enumerate() {
                    *level = ((encoded >> (2 * i)) & 3) as u8;
                }
                let order = visual_order(&levels);
                let mut sorted = order.clone();
                sorted.sort_unstable();
                assert_eq!(sorted, (0..len).collect::<Vec<_>>(), "{levels:?}");

                if levels.iter().all(|level| level % 2 == 0) {
                    assert_eq!(order, sorted, "{levels:?} has no odd level to reorder for");
                }

                // Items at the same level stay contiguous: a reversal moves a maximal window, so it
                // can never interleave two runs that were separate.
                for window in order.windows(2) {
                    let (a, b) = (window[0], window[1]);
                    if levels[a] == levels[b] {
                        assert_eq!(
                            a.abs_diff(b),
                            1,
                            "{levels:?} split a same-level pair: {order:?}"
                        );
                    }
                }
            }
        }
    }

    /// The maximum embedding depth UAX #9 allows, so the sentinel is provably above it.
    ///
    /// Only the level-125 pair moves: 124 is *even*, so those two items are left-to-right and the
    /// loop's single pass never reaches a window containing them.
    #[test]
    fn the_deepest_representable_embedding_still_reorders() {
        assert_eq!(visual_order(&[124, 125, 125, 124]), vec![0, 2, 1, 3]);
        assert_eq!(visual_order(&[125, 125, 125]), vec![2, 1, 0]);
        assert_eq!(visual_order(&[u8::MAX]), vec![0]);
    }

    #[test]
    fn reorder_in_place_agrees_with_the_allocating_form() {
        let levels = [0u8, 1, 2, 2, 1, 0];
        let mut order: Vec<usize> = (0..levels.len()).collect();
        reorder(&levels, &mut order);
        assert_eq!(order, visual_order(&levels));
    }
}
