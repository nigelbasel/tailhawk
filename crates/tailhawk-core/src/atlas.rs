//! The glyph atlas — `SPEC.md` §3.2's fixed-slot LRU.
//!
//! This is the bookkeeping half of V2 and it touches no device: which glyph lives in which cell of
//! a sheet, which cell is given up when the sheet is full, and which glyphs are known to have no
//! ink at all. Rasterising into those cells is DirectWrite's job and lives elsewhere.
//!
//! **Every rule here was measured rather than reasoned about**, in `experiments/g4-glyph-atlas`:
//!
//! - **Uniform slots, one glyph each.** Not a shelf packer and not variable-width spans. A variant
//!   that let a glyph span adjacent slots cost **106 ms per frame** in eviction bookkeeping,
//!   because the victim search became O(capacity × span) and had to find a free *run*.
//! - **An O(1) victim list.** Scanning every slot for the oldest cost **4–9 ms per frame** under
//!   thrashing — up to half a 60 Hz budget before anything is drawn — against **0.17–0.37 ms** for
//!   an intrusive doubly-linked list. 20–28x in every run.
//! - **A slot touched in the current frame is never evicted**, or the frame corrupts itself by
//!   overwriting a cell it is about to draw from.
//! - **Cache the absence of ink.** A space, or a codepoint the face does not have, is remembered as
//!   blank and occupies no slot. Without this every space is a miss on every frame: G4's first run
//!   produced exactly 440 spurious misses per frame from ten spaces per row.
//!
//! The cost of uniform slots is density — roughly 46% of the sheet goes to padding around narrow
//! Latin glyphs — which is a good trade for a monospace grid where widths are nearly uniform.

use std::collections::HashMap;

/// Which face a glyph id belongs to. Glyph ids are face-local, so this is part of the key rather
/// than a convenience: font fallback (§3.3) means one viewport draws from several faces at once,
/// and glyph 42 of the CJK fallback is not glyph 42 of the primary.
pub type FaceId = u16;

/// A DirectWrite glyph index. Not a character, and not a cluster — the cell model in
/// [`crate::cell`] decides what a cluster is, shaping decides which glyphs draw it, and only then
/// does the atlas see anything.
pub type GlyphId = u16;

/// An index into the sheet's slots. Stable while the glyph is resident and reused the moment it is
/// evicted, which is why nothing outside a frame may hold one.
pub type SlotId = u32;

const NIL: SlotId = SlotId::MAX;

/// Emboldening or slanting applied *on top of* a face, rather than by choosing a different face.
///
/// Bold and italic are normally separate faces, and a face is already in the key. What is not is a
/// synthetic variant of the same face — the same glyph id, the same size, a different raster — so
/// leaving it out would show the upright glyph where the slanted one belongs.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Synthetic {
    pub bold: bool,
    pub oblique: bool,
}

/// `SPEC.md` §3.2's atlas key: "(glyph id, style, dpi scale)", with the face made explicit.
///
/// The size is in **whole device pixels**, not points and not a scale factor. §3.2 requires column
/// advances to be computed in integer device pixels at the current scale, because fractional
/// per-glyph rounding accumulates drift and visibly misaligns columns across a wide window; a key
/// that carried a float would be able to hold two raster sizes that round to the same cell.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub face: FaceId,
    pub glyph: GlyphId,
    pub px_per_em: u16,
    pub synthetic: Synthetic,
}

/// A glyph's ink: how big its raster is, and where that raster sits relative to the pen position
/// on the baseline. Both come from DirectWrite's measured bounds.
///
/// **Never derive this from the em size.** `experiments/g4b-batched-raster` guessed a 20×26 cell
/// for em 14 and clipped 1,086 of 1,500 glyphs — and the clipped cells then compared *equal*
/// between two rasterisation strategies, which would have been read as a correctness pass.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Ink {
    pub width: u16,
    pub height: u16,
    pub left: i16,
    pub top: i16,
}

/// Where a resident glyph is, and how to place it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub slot: SlotId,
    /// Top-left of the slot within the sheet, in texels.
    pub x: u16,
    pub y: u16,
    pub ink: Ink,
}

/// What the atlas knows about a glyph right now.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Residency {
    /// Draw it from here.
    Resident(Placement),
    /// It has no ink. Draw nothing, and do not ask for it to be rasterised again.
    Blank,
    /// Not here. Rasterise it off the paint path and draw a placeholder in the meantime —
    /// `SPEC.md` §3.2: a frame must never block on rasterisation.
    Absent,
    /// Too big for a slot, and permanently so. Draw the placeholder and **do not ask again**.
    ///
    /// Uniform slots are what make eviction O(1), so a glyph wider or taller than the cell — box
    /// drawing, a block, an accented capital, anything East Asian Wide — cannot be taken in.
    /// Until this existed such a glyph stayed [`Absent`](Residency::Absent) and was re-rasterised
    /// on every single frame, for ever, at the cost of a whole batch each time.
    ///
    /// **The placeholder is a known limit, not the intended end state.** Drawing these properly
    /// wants a second sheet with taller slots, or a variable-size atlas, and that is a larger
    /// change than making the corruption stop.
    Oversized,
}

/// Why a raster could not be taken in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum InsertError {
    /// The ink does not fit a slot. Uniform slots are what make eviction O(1), so the answer is to
    /// draw this glyph some other way, not to make the atlas variable-width.
    TooLarge,
    /// Every slot in the sheet is being drawn from in *this* frame, so evicting any of them would
    /// corrupt the frame in progress. Draw a placeholder; the slot will be free next frame.
    ///
    /// A sheet too small to hold even one slot reports this too, and for that sheet it never
    /// stops being true. That is a configuration error rather than a state to recover from, and
    /// the response is the same either way: draw the placeholder, do not panic.
    SheetFullThisFrame,
}

#[derive(Copy, Clone)]
struct Slot {
    prev: SlotId,
    next: SlotId,
    key: Option<GlyphKey>,
    ink: Ink,
    last_used: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Entry {
    Slot(SlotId),
    Blank,
    /// Refused for being too big for a slot. Terminal, like `Blank`: without it the glyph stays
    /// `Absent`, is queued again on the next frame, and is rasterised for ever at the cost of a
    /// full batch each time.
    Oversized,
}

pub struct Atlas {
    slots: Vec<Slot>,
    map: HashMap<GlyphKey, Entry>,
    /// Least-recently-used first. Vacant slots start at the head, so allocation and eviction are
    /// the same operation and neither needs a search.
    head: SlotId,
    tail: SlotId,
    cols: u16,
    slot_w: u16,
    slot_h: u16,
    frame: u64,
    blanks: usize,
    blank_limit: usize,
}

impl Atlas {
    /// Lays a sheet out into whole slots of the given size. Any remainder at the right and bottom
    /// edges is unused — a partial slot is not a slot.
    ///
    /// **The slot size belongs to the caller and must come from measured glyph bounds**, wide
    /// enough for the widest glyph the atlas is expected to accept; see [`Ink`].
    pub fn new(sheet_w: u16, sheet_h: u16, slot_w: u16, slot_h: u16) -> Self {
        let cols = sheet_w.checked_div(slot_w).unwrap_or(0);
        let rows = sheet_h.checked_div(slot_h).unwrap_or(0);
        let capacity = cols as usize * rows as usize;

        let mut slots = Vec::with_capacity(capacity);
        for i in 0..capacity {
            slots.push(Slot {
                prev: if i == 0 { NIL } else { i as SlotId - 1 },
                next: if i + 1 == capacity {
                    NIL
                } else {
                    i as SlotId + 1
                },
                key: None,
                ink: Ink::default(),
                last_used: 0,
            });
        }

        Self {
            head: if capacity == 0 { NIL } else { 0 },
            tail: if capacity == 0 {
                NIL
            } else {
                capacity as SlotId - 1
            },
            slots,
            map: HashMap::new(),
            cols,
            slot_w,
            slot_h,
            // Blanks cost no slot, so nothing else bounds them, and a viewer left open on a log
            // full of unusual codepoints would grow the map for ever. The whole blank set is
            // cheap to rebuild — a blank is one miss, not a rasterisation — so it is dropped
            // wholesale rather than given an LRU of its own.
            blank_limit: (capacity * 4).max(1024),
            frame: 1,
            blanks: 0,
        }
    }

    /// How many glyphs the sheet holds. Not the sheet's area: a sheet that does not divide evenly
    /// by the slot size holds fewer.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many slots are occupied. Blanks are not counted — they hold no slot.
    pub fn len(&self) -> usize {
        self.map.len() - self.blanks
    }

    /// How many blank-or-oversized entries are held. Diagnostic.
    pub fn blank_count(&self) -> usize {
        self.blanks
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Starts a new frame. Everything used since the last call becomes evictable again.
    pub fn begin_frame(&mut self) {
        self.frame += 1;
    }

    /// Looks a glyph up and, if it is there, records that this frame is using it — which is both
    /// what makes the policy LRU rather than FIFO and what protects the slot from being evicted
    /// out from under the frame drawing it.
    pub fn lookup(&mut self, key: &GlyphKey) -> Residency {
        match self.map.get(key) {
            None => Residency::Absent,
            Some(Entry::Blank) => Residency::Blank,
            Some(Entry::Oversized) => Residency::Oversized,
            Some(&Entry::Slot(slot)) => {
                self.touch(slot);
                Residency::Resident(self.placement(slot))
            }
        }
    }

    /// Takes a freshly rasterised glyph in, evicting the least recently used slot if the sheet is
    /// full. The returned placement says where to upload the raster.
    pub fn insert(&mut self, key: GlyphKey, ink: Ink) -> Result<Placement, InsertError> {
        if ink.width > self.slot_w || ink.height > self.slot_h {
            return Err(InsertError::TooLarge);
        }
        // A raster can arrive for a glyph that is already here — two frames can both miss on it
        // before either rasterisation finishes. Taking it in twice would spend two slots on one
        // glyph and leak the first.
        if let Some(&Entry::Slot(slot)) = self.map.get(&key) {
            self.touch(slot);
            return Ok(self.placement(slot));
        }

        let slot = self.head;
        if slot == NIL {
            return Err(InsertError::SheetFullThisFrame);
        }
        // The head is the least recently used slot, so if *it* was used this frame then every slot
        // was, and there is nothing that can be given up without corrupting the frame.
        if self.slots[slot as usize].last_used == self.frame {
            return Err(InsertError::SheetFullThisFrame);
        }

        if let Some(evicted) = self.slots[slot as usize].key.take() {
            self.map.remove(&evicted);
        }
        self.slots[slot as usize].key = Some(key);
        self.slots[slot as usize].ink = ink;
        self.map.insert(key, Entry::Slot(slot));
        self.touch(slot);
        Ok(self.placement(slot))
    }

    /// Records that a glyph has no raster at all, so it is never asked for again.
    /// Records that a glyph is too big to ever be taken in, so it is not asked for again.
    ///
    /// Counted against the same ceiling as blanks and swept the same way: both are entries that
    /// hold no slot, and a hostile file full of distinct oversized codepoints must not grow the map
    /// without bound any more than one full of distinct spaces may.
    pub fn insert_oversized(&mut self, key: GlyphKey) {
        if self.blanks >= self.blank_limit {
            self.map
                .retain(|_, e| *e != Entry::Blank && *e != Entry::Oversized);
            self.blanks = 0;
        }
        if self.map.insert(key, Entry::Oversized) != Some(Entry::Oversized) {
            self.blanks += 1;
        }
    }

    pub fn insert_blank(&mut self, key: GlyphKey) {
        if self.blanks >= self.blank_limit {
            self.map.retain(|_, e| *e != Entry::Blank);
            self.blanks = 0;
        }
        if self.map.insert(key, Entry::Blank) != Some(Entry::Blank) {
            self.blanks += 1;
        }
    }

    /// Drops everything. `SPEC.md` §3.2 rebuilds the atlas per scale factor, and a device rebuilt
    /// after `DXGI_ERROR_DEVICE_REMOVED` owns a new, empty sheet.
    pub fn clear(&mut self) {
        self.map.clear();
        self.blanks = 0;
        let capacity = self.slots.len();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            slot.key = None;
            slot.ink = Ink::default();
            slot.last_used = 0;
            slot.prev = if i == 0 { NIL } else { i as SlotId - 1 };
            slot.next = if i + 1 == capacity {
                NIL
            } else {
                i as SlotId + 1
            };
        }
        self.head = if capacity == 0 { NIL } else { 0 };
        self.tail = if capacity == 0 {
            NIL
        } else {
            capacity as SlotId - 1
        };
    }

    fn placement(&self, slot: SlotId) -> Placement {
        let col = slot % self.cols.max(1) as u32;
        let row = slot / self.cols.max(1) as u32;
        Placement {
            slot,
            x: col as u16 * self.slot_w,
            y: row as u16 * self.slot_h,
            ink: self.slots[slot as usize].ink,
        }
    }

    /// Moves a slot to the most-recently-used end. Two pointer writes and no search, which is the
    /// whole reason the list exists.
    fn touch(&mut self, slot: SlotId) {
        self.slots[slot as usize].last_used = self.frame;
        if self.tail == slot {
            return;
        }
        self.unlink(slot);
        let old_tail = self.tail;
        self.slots[slot as usize].prev = old_tail;
        self.slots[slot as usize].next = NIL;
        if old_tail != NIL {
            self.slots[old_tail as usize].next = slot;
        } else {
            self.head = slot;
        }
        self.tail = slot;
    }

    fn unlink(&mut self, slot: SlotId) {
        let (prev, next) = {
            let s = &self.slots[slot as usize];
            (s.prev, s.next)
        };
        if prev != NIL {
            self.slots[prev as usize].next = next;
        } else {
            self.head = next;
        }
        if next != NIL {
            self.slots[next as usize].prev = prev;
        } else {
            self.tail = prev;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(glyph: GlyphId) -> GlyphKey {
        GlyphKey {
            face: 0,
            glyph,
            px_per_em: 14,
            synthetic: Synthetic::default(),
        }
    }

    fn ink() -> Ink {
        Ink {
            width: 8,
            height: 12,
            left: 0,
            top: -11,
        }
    }

    /// A sheet that does not divide evenly by the slot size holds whole slots only. Reading the
    /// leftover strip as capacity would place a glyph partly outside the sheet.
    #[test]
    fn capacity_is_whole_slots_not_sheet_area() {
        let atlas = Atlas::new(100, 100, 30, 30);
        assert_eq!(atlas.capacity(), 9);
        assert_eq!(Atlas::new(64, 64, 16, 16).capacity(), 16);
        assert_eq!(Atlas::new(8, 8, 16, 16).capacity(), 0);
    }

    #[test]
    fn a_resident_glyph_is_found_again_rather_than_rasterised_again() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        assert_eq!(atlas.lookup(&key(1)), Residency::Absent);
        let placed = atlas.insert(key(1), ink()).expect("room in an empty sheet");
        assert_eq!(atlas.lookup(&key(1)), Residency::Resident(placed));
        assert_eq!(atlas.len(), 1);
    }

    #[test]
    fn every_slot_is_reachable_and_none_of_them_overlap() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        let mut seen = Vec::new();
        for g in 0..atlas.capacity() as GlyphId {
            let p = atlas.insert(key(g), ink()).expect("within capacity");
            assert!(
                p.x + 16 <= 64 && p.y + 16 <= 64,
                "slot {p:?} leaves the sheet"
            );
            seen.push((p.x, p.y));
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), atlas.capacity(), "two glyphs shared a cell");
    }

    /// The distinction between LRU and FIFO, and the reason `lookup` takes `&mut self`: the glyph
    /// nobody asked for is the one that goes, not the one that arrived first.
    #[test]
    fn the_glyph_nobody_asked_for_is_the_one_evicted() {
        let mut atlas = Atlas::new(32, 16, 16, 16);
        assert_eq!(atlas.capacity(), 2);
        atlas.insert(key(1), ink()).unwrap();
        atlas.insert(key(2), ink()).unwrap();

        atlas.begin_frame();
        atlas.lookup(&key(1));

        atlas.begin_frame();
        atlas
            .insert(key(3), ink())
            .expect("something can be given up");

        assert_eq!(
            atlas.lookup(&key(1)),
            Residency::Resident(_placement(&atlas, 1))
        );
        assert_eq!(
            atlas.lookup(&key(2)),
            Residency::Absent,
            "the glyph that went unasked-for should have been the victim"
        );
    }

    fn _placement(atlas: &Atlas, glyph: GlyphId) -> Placement {
        match atlas.map.get(&key(glyph)) {
            Some(&Entry::Slot(slot)) => atlas.placement(slot),
            other => panic!("glyph {glyph} is {other:?}, not resident"),
        }
    }

    /// The rule that stops a frame corrupting itself: a cell being drawn from right now cannot be
    /// overwritten by a glyph arriving later in the same frame.
    #[test]
    fn a_slot_used_in_this_frame_is_never_evicted() {
        let mut atlas = Atlas::new(32, 16, 16, 16);
        atlas.insert(key(1), ink()).unwrap();
        atlas.insert(key(2), ink()).unwrap();
        assert_eq!(
            atlas.insert(key(3), ink()),
            Err(InsertError::SheetFullThisFrame),
            "both slots belong to this frame"
        );
        assert_eq!(
            atlas.lookup(&key(1)),
            Residency::Resident(_placement(&atlas, 1)),
            "the glyph the frame is drawing must still be where it was"
        );

        atlas.begin_frame();
        assert!(
            atlas.insert(key(3), ink()).is_ok(),
            "next frame the same slots are fair game"
        );
    }

    /// **A glyph too big for a slot is remembered as such, and never asked for again.**
    ///
    /// The `TooLarge` refusal was unreachable for the life of the atlas: `cut_cell` reported every
    /// glyph's ink as the uniform cell, so the check compared the cell against slot dimensions
    /// derived from that same cell. It read as a safety net and could not fire. Now that the
    /// rasteriser reports an overflowing glyph honestly, this is the state such a glyph lands in —
    /// and the point of the state is the second assertion: left `Absent`, it would be rasterised
    /// again on every frame for the life of the process.
    #[test]
    fn a_glyph_too_big_for_a_slot_is_refused_once_and_not_asked_for_again() {
        let mut atlas = Atlas::new(32, 16, 16, 16);
        let oversized = Ink {
            width: 17,
            height: 16,
            left: 0,
            top: 0,
        };
        assert_eq!(atlas.insert(key(1), oversized), Err(InsertError::TooLarge));
        assert_eq!(
            atlas.lookup(&key(1)),
            Residency::Absent,
            "refusing alone does not record anything — that is what left it re-queued for ever"
        );
        atlas.insert_oversized(key(1));
        assert_eq!(atlas.lookup(&key(1)), Residency::Oversized);
        assert_eq!(atlas.len(), 0, "and it must not spend a slot");
    }

    /// A file full of distinct oversized codepoints must not grow the map without bound, for the
    /// same reason a file full of distinct blanks must not.
    #[test]
    fn oversized_entries_are_swept_like_blanks() {
        let mut atlas = Atlas::new(32, 16, 16, 16);
        for g in 0..10_000u16 {
            atlas.insert_oversized(key(g));
        }
        assert!(
            atlas.map.len() <= atlas.blank_limit + 1,
            "the map grew to {} entries",
            atlas.map.len()
        );
    }

    /// G4's first run produced exactly 440 spurious misses per frame — ten spaces per row on a
    /// 44-row fixture — because a glyph with no raster was never recorded as having none.
    #[test]
    fn a_glyph_with_no_ink_is_remembered_and_costs_no_slot() {
        let mut atlas = Atlas::new(32, 16, 16, 16);
        atlas.insert_blank(key(32));
        assert_eq!(atlas.lookup(&key(32)), Residency::Blank);
        assert_eq!(atlas.len(), 0, "a blank must not spend a slot");

        atlas.insert(key(1), ink()).unwrap();
        atlas.insert(key(2), ink()).unwrap();
        assert_eq!(
            atlas.lookup(&key(32)),
            Residency::Blank,
            "a full sheet does not forget what has no ink"
        );
    }

    #[test]
    fn a_glyph_too_big_for_a_slot_is_refused_rather_than_overflowing_its_neighbour() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        let too_wide = Ink {
            width: 17,
            height: 12,
            left: 0,
            top: -11,
        };
        let too_tall = Ink {
            width: 8,
            height: 17,
            left: 0,
            top: -11,
        };
        assert_eq!(atlas.insert(key(1), too_wide), Err(InsertError::TooLarge));
        assert_eq!(atlas.insert(key(2), too_tall), Err(InsertError::TooLarge));
        assert_eq!(atlas.len(), 0);
        let exact = Ink {
            width: 16,
            height: 16,
            left: 0,
            top: -11,
        };
        assert!(
            atlas.insert(key(3), exact).is_ok(),
            "a glyph that exactly fills its slot fits"
        );
    }

    /// Two frames can both miss on the same glyph before either rasterisation lands.
    #[test]
    fn a_glyph_that_arrives_twice_spends_one_slot() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        let first = atlas.insert(key(1), ink()).unwrap();
        let second = atlas.insert(key(1), ink()).unwrap();
        assert_eq!(first, second);
        assert_eq!(atlas.len(), 1);
    }

    #[test]
    fn eviction_forgets_the_victim_rather_than_leaving_it_pointing_at_a_reused_slot() {
        let mut atlas = Atlas::new(16, 16, 16, 16);
        assert_eq!(atlas.capacity(), 1);
        atlas.insert(key(1), ink()).unwrap();
        atlas.begin_frame();
        atlas.insert(key(2), ink()).unwrap();
        assert_eq!(
            atlas.lookup(&key(1)),
            Residency::Absent,
            "the evicted glyph would otherwise be drawn from a cell that now holds another glyph"
        );
        assert_eq!(atlas.len(), 1);
    }

    /// Every part of the key has to be part of the key. An 'A' at 12 px drawn from the 24 px cell
    /// is the bug this prevents, and the same goes for a fallback face's glyph 42 and for a
    /// synthetically slanted variant of a face that is otherwise identical.
    #[test]
    fn each_part_of_the_key_distinguishes_a_different_raster() {
        let base = key(65);
        let variants = [
            GlyphKey {
                px_per_em: 28,
                ..base
            },
            GlyphKey { face: 1, ..base },
            GlyphKey { glyph: 66, ..base },
            GlyphKey {
                synthetic: Synthetic {
                    bold: true,
                    oblique: false,
                },
                ..base
            },
            GlyphKey {
                synthetic: Synthetic {
                    bold: false,
                    oblique: true,
                },
                ..base
            },
        ];
        let mut atlas = Atlas::new(64, 64, 16, 16);
        atlas.insert(base, ink()).unwrap();
        for (i, variant) in variants.iter().enumerate() {
            assert_eq!(
                atlas.lookup(variant),
                Residency::Absent,
                "variant {i} ({variant:?}) collided with {base:?}"
            );
            atlas.insert(*variant, ink()).unwrap();
        }
        assert_eq!(atlas.len(), 1 + variants.len());
    }

    /// `SPEC.md` §3.2 rebuilds the atlas per scale factor, and a device rebuilt after a removal
    /// owns a new sheet with nothing in it.
    #[test]
    fn clearing_empties_the_slots_and_the_blanks_together() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        atlas.insert(key(1), ink()).unwrap();
        atlas.insert_blank(key(32));
        atlas.clear();

        assert_eq!(atlas.len(), 0);
        assert_eq!(atlas.lookup(&key(1)), Residency::Absent);
        assert_eq!(
            atlas.lookup(&key(32)),
            Residency::Absent,
            "a blank at the old scale says nothing about the new one"
        );
        for g in 0..atlas.capacity() as GlyphId {
            atlas
                .insert(key(g), ink())
                .expect("the whole sheet is free");
        }
    }

    /// A blank costs no slot, so nothing else bounds the set. A viewer left open for days on logs
    /// full of unusual codepoints would otherwise grow the map without limit.
    #[test]
    fn the_blank_set_is_bounded() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        let limit = atlas.blank_limit;
        for g in 0..(limit as u32 * 2 + 5) {
            atlas.insert_blank(GlyphKey {
                face: (g >> 16) as u16,
                glyph: g as u16,
                px_per_em: 14,
                synthetic: Synthetic::default(),
            });
        }
        assert!(
            atlas.map.len() <= limit,
            "{} blanks retained against a limit of {limit}",
            atlas.map.len()
        );
    }

    #[test]
    fn re_blanking_the_same_glyph_is_not_a_new_blank() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        for _ in 0..10 {
            atlas.insert_blank(key(32));
        }
        assert_eq!(atlas.blanks, 1);
        assert_eq!(atlas.map.len(), 1);
    }

    /// A sheet too small for even one slot is a configuration error, not a panic: it reports a
    /// full sheet and the renderer draws placeholders.
    #[test]
    fn a_sheet_with_no_room_for_a_slot_refuses_rather_than_panicking() {
        let mut atlas = Atlas::new(8, 8, 16, 16);
        assert_eq!(atlas.capacity(), 0);
        assert_eq!(
            atlas.insert(key(1), ink()),
            Err(InsertError::SheetFullThisFrame),
            "the ink fits a slot; there is simply nowhere to put a slot"
        );
        atlas.begin_frame();
        assert_eq!(
            atlas.insert(key(1), ink()),
            Err(InsertError::SheetFullThisFrame),
            "and no frame ever makes room on a sheet with no slots"
        );
        assert_eq!(atlas.lookup(&key(1)), Residency::Absent);
    }

    /// The thrashing case G4 measured, driven end to end: every frame asks for a working set
    /// larger than the sheet, and the atlas must keep serving without losing track of a slot.
    #[test]
    fn a_working_set_larger_than_the_sheet_keeps_its_bookkeeping_straight() {
        let mut atlas = Atlas::new(64, 64, 16, 16);
        let capacity = atlas.capacity();
        for frame in 0..50u16 {
            atlas.begin_frame();
            for g in 0..capacity as u16 {
                let k = key(frame * capacity as u16 + g);
                if atlas.lookup(&k) == Residency::Absent {
                    atlas.insert(k, ink()).expect("a fresh frame can evict");
                }
            }
            assert_eq!(atlas.len(), capacity, "frame {frame} lost a slot");
        }

        let mut occupied: Vec<SlotId> = atlas
            .map
            .values()
            .map(|e| match e {
                Entry::Slot(s) => *s,
                Entry::Blank | Entry::Oversized => {
                    unreachable!("this test inserts only real rasters")
                }
            })
            .collect();
        occupied.sort_unstable();
        occupied.dedup();
        assert_eq!(
            occupied.len(),
            capacity,
            "two keys ended up pointing at one slot"
        );
    }
}
