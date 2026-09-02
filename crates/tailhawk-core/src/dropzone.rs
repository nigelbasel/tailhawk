//! Where a dragged tab would land, and the shape drawn to say so.
//!
//! `SPEC.md` §1069 asks for **drag-out-to-split**: pull a tab off the strip, and the pane it is
//! over offers to divide. This is the whole of that decision — a rectangle and a pointer in, the
//! zone and its guide out — so the part that can be got wrong is the part a test can reach. The
//! shell's job is to draw the guide it is handed and act on the drop.
//!
//! **All five zones are performable, as of 2026-09-02.** They were not always: a `Tab` used to hold
//! `panes: Vec<Document>` divided by a *height* alone, so a pane could be put above or below
//! another and could not be put beside one, and this module carried a `Zone::available` that the
//! shell asked before drawing any guide — because a highlight is a promise, and one the product
//! cannot keep is the dead menu item this project has already shipped once. `Tab` now carries the
//! direction it divides in, so `available` has gone rather than becoming a function that always
//! answers yes.
//!
//! **What still refuses a drop is the pane *count*, not its direction.** The model says how two
//! panes share a space and not how three do, so a drop onto an already-split pane is declined by
//! the shell — for all four edges equally.

/// A rectangle in the coordinates the caller is working in. Pixels, in practice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    /// Does this rectangle contain the point?
    pub fn holds(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// What the pointer is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// Put the dragged document above the one being hovered.
    Above,
    /// Put it below.
    Below,
    /// Beside it, to the left. **Not available yet** — see the module note.
    Left,
    /// Beside it, to the right. **Not available yet.**
    Right,
    /// No split: just show that document. What a drag that ends where it began should do.
    Centre,
}

impl Zone {
    /// Whether taking this zone divides the pane at all.
    pub fn splits(self) -> bool {
        !matches!(self, Zone::Centre)
    }
}

/// How much of the pane's shorter side each edge band takes.
///
/// A quarter, which is the proportion the editors that popularised this gesture use, and it has to
/// be a *proportion*: a fixed band is unusable in a short pane and a rounding error in a tall one.
const BAND: f32 = 0.25;

/// The largest an edge band gets, so a very large pane does not turn into four enormous targets
/// with a pinhole in the middle.
const BAND_MAX: f32 = 120.0;

/// Which zone the pointer is over, and the rectangle to highlight for it.
///
/// `None` when the pointer is outside the pane entirely — the caller is over something else, and
/// nothing should be drawn.
///
/// **The bands are measured from the shorter side.** Taking each side's own length would make the
/// top band of a wide, short pane a different thickness from its left band, so the target you hit
/// would depend on which way you approached the middle.
pub fn at(pane: Rect, x: f32, y: f32) -> Option<(Zone, Rect)> {
    if !pane.holds(x, y) || pane.w <= 0.0 || pane.h <= 0.0 {
        return None;
    }
    let band = (pane.w.min(pane.h) * BAND).min(BAND_MAX);
    let (dx, dy) = (x - pane.x, y - pane.y);
    let (from_right, from_bottom) = (pane.w - dx, pane.h - dy);

    // The nearest edge wins, so the diagonals divide evenly and there is no wedge that belongs to
    // two zones or to none.
    let nearest = dx.min(from_right).min(dy).min(from_bottom);
    let zone = if nearest >= band {
        Zone::Centre
    } else if nearest == dy {
        Zone::Above
    } else if nearest == from_bottom {
        Zone::Below
    } else if nearest == dx {
        Zone::Left
    } else {
        Zone::Right
    };
    Some((zone, guide(pane, zone)))
}

/// The rectangle a zone highlights: the half the dragged document would occupy, or the whole pane
/// for [`Zone::Centre`].
///
/// Half rather than a thin strip at the edge, because the guide's job is to answer "where will it
/// go", and the honest answer is the space it will actually take.
pub fn guide(pane: Rect, zone: Zone) -> Rect {
    let (half_w, half_h) = (pane.w / 2.0, pane.h / 2.0);
    match zone {
        Zone::Centre => pane,
        Zone::Above => Rect { h: half_h, ..pane },
        Zone::Below => Rect {
            y: pane.y + half_h,
            h: pane.h - half_h,
            ..pane
        },
        Zone::Left => Rect { w: half_w, ..pane },
        Zone::Right => Rect {
            x: pane.x + half_w,
            w: pane.w - half_w,
            ..pane
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: Rect = Rect {
        x: 100.0,
        y: 50.0,
        w: 800.0,
        h: 400.0,
    };

    fn zone_at(x: f32, y: f32) -> Option<Zone> {
        at(PANE, x, y).map(|(z, _)| z)
    }

    #[test]
    fn each_edge_claims_its_own_band_and_the_middle_is_no_split() {
        // band = min(800, 400) * 0.25 = 100.
        assert_eq!(
            zone_at(500.0, 60.0),
            Some(Zone::Above),
            "just inside the top"
        );
        assert_eq!(zone_at(500.0, 440.0), Some(Zone::Below));
        assert_eq!(zone_at(110.0, 250.0), Some(Zone::Left));
        assert_eq!(zone_at(890.0, 250.0), Some(Zone::Right));
        assert_eq!(zone_at(500.0, 250.0), Some(Zone::Centre), "the middle");
    }

    /// A point outside the pane belongs to nobody. The shell uses this to decide whether to draw a
    /// guide at all, so answering `Centre` for a pointer that has left the window would paint a
    /// full-pane highlight over a pane the user is not pointing at.
    #[test]
    fn a_pointer_outside_the_pane_asks_for_nothing() {
        assert_eq!(zone_at(50.0, 250.0), None, "left of it");
        assert_eq!(zone_at(950.0, 250.0), None);
        assert_eq!(zone_at(500.0, 10.0), None, "above it");
        assert_eq!(zone_at(500.0, 500.0), None);
        assert_eq!(
            at(PANE, 100.0, 50.0).map(|(z, _)| z),
            Some(Zone::Above),
            "the top-left corner is inside"
        );
    }

    /// **Every point inside the pane belongs to exactly one zone.** A wedge owned by two zones
    /// flickers between them as the pointer moves; one owned by none leaves a hole where the guide
    /// vanishes. Walking the pane is the only way to know neither happens.
    #[test]
    fn every_point_in_the_pane_has_exactly_one_answer() {
        let mut seen = [0usize; 5];
        let mut x = PANE.x;
        while x < PANE.x + PANE.w {
            let mut y = PANE.y;
            while y < PANE.y + PANE.h {
                let zone = zone_at(x, y).expect("inside the pane, so some zone owns it");
                seen[match zone {
                    Zone::Above => 0,
                    Zone::Below => 1,
                    Zone::Left => 2,
                    Zone::Right => 3,
                    Zone::Centre => 4,
                }] += 1;
                y += 3.0;
            }
            x += 3.0;
        }
        assert!(
            seen.iter().all(|&n| n > 0),
            "some zone is unreachable: {seen:?}"
        );
    }

    /// The bands come off the **shorter** side, so approaching the middle from the top and from the
    /// side crosses the boundary at the same distance. Measured per-side they would not, and the
    /// target would depend on the direction of approach.
    #[test]
    fn the_bands_are_square_even_when_the_pane_is_not() {
        let wide = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 200.0,
        };
        // band = min(1000, 200) * 0.25 = 50, so 49 in is an edge and 51 in is not.
        assert_eq!(at(wide, 500.0, 49.0).map(|(z, _)| z), Some(Zone::Above));
        assert_eq!(at(wide, 500.0, 51.0).map(|(z, _)| z), Some(Zone::Centre));
        assert_eq!(at(wide, 49.0, 100.0).map(|(z, _)| z), Some(Zone::Left));
        assert_eq!(at(wide, 51.0, 100.0).map(|(z, _)| z), Some(Zone::Centre));
    }

    /// A pane large enough that a quarter of it would be an absurd target keeps its bands sane.
    #[test]
    fn a_very_large_pane_does_not_get_enormous_edges() {
        let huge = Rect {
            x: 0.0,
            y: 0.0,
            w: 4000.0,
            h: 3000.0,
        };
        assert_eq!(at(huge, 2000.0, 130.0).map(|(z, _)| z), Some(Zone::Centre));
        assert_eq!(at(huge, 2000.0, 110.0).map(|(z, _)| z), Some(Zone::Above));
    }

    /// **The guide shows the space the document would actually take**, which is what makes it an
    /// answer to "where will this go" rather than a decoration.
    #[test]
    fn the_guide_is_the_half_the_document_would_occupy() {
        assert_eq!(
            guide(PANE, Zone::Above),
            Rect {
                x: 100.0,
                y: 50.0,
                w: 800.0,
                h: 200.0
            }
        );
        assert_eq!(
            guide(PANE, Zone::Below),
            Rect {
                x: 100.0,
                y: 250.0,
                w: 800.0,
                h: 200.0
            }
        );
        assert_eq!(
            guide(PANE, Zone::Left),
            Rect {
                x: 100.0,
                y: 50.0,
                w: 400.0,
                h: 400.0
            }
        );
        assert_eq!(
            guide(PANE, Zone::Centre),
            PANE,
            "no split, so the whole pane"
        );
    }

    /// The two halves of a split must tile the pane exactly — no overlap, no seam — or the split
    /// loses or duplicates a row of pixels.
    #[test]
    fn the_two_halves_of_a_split_cover_the_pane_exactly() {
        let odd = Rect {
            x: 0.0,
            y: 0.0,
            w: 801.0,
            h: 401.0,
        };
        let (top, bottom) = (guide(odd, Zone::Above), guide(odd, Zone::Below));
        assert_eq!(
            top.y + top.h,
            bottom.y,
            "a seam or an overlap between the halves"
        );
        assert_eq!(
            bottom.y + bottom.h,
            odd.y + odd.h,
            "the bottom half must reach the edge"
        );
        let (left, right) = (guide(odd, Zone::Left), guide(odd, Zone::Right));
        assert_eq!(left.x + left.w, right.x);
        assert_eq!(right.x + right.w, odd.x + odd.w);
    }

    #[test]
    fn only_the_centre_leaves_the_pane_undivided() {
        assert!(!Zone::Centre.splits());
        for zone in [Zone::Above, Zone::Below, Zone::Left, Zone::Right] {
            assert!(zone.splits(), "{zone:?}");
        }
    }
}
