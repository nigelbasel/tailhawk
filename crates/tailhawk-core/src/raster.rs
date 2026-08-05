//! Glyph rasterisation leaf backend — DirectWrite into atlas-sized cells.
//!
//! The other half of V2. [`crate::atlas`] decides *where* a glyph lives; this decides what the
//! pixels are. It needs no D3D device and no window, which is why it is testable on its own.
//!
//! **Three things here are measured rather than chosen**, in `experiments/g4b-batched-raster`:
//!
//! - **Rasterise 4–64 glyphs per `CreateGlyphRunAnalysis`.** Batched output is *bit-identical* to
//!   per-glyph output at any inter-glyph gap including zero, and worth ~1.8x on the cold path. The
//!   win saturates at 4, and past 256 glyphs per analysis it becomes a **pessimisation** — a whole
//!   viewport in one analysis is slower than sixteen at a time.
//! - **Derive the cell from measured bounds, never from the em size.** Guessing 20×26 for em 14
//!   clipped 1,086 of 1,500 glyphs, and the clipped cells then compared *equal* between two
//!   rasterisation strategies — which would have been read as a correctness pass.
//! - **The cost that matters is not ours.** DirectWrite is backed by a cross-process system font
//!   cache: ~86–108 µs for the first process on a machine to rasterise a given (glyph, size), ~3 µs
//!   for every process afterwards, surviving process exit. So the expensive case is first run on a
//!   machine, not steady state, and no amount of local caching addresses it.
//!
//! Only the monochrome path is here. Colour glyphs go through `TranslateColorGlyphRun` into a
//! second sheet (`SPEC.md` §3.3) and are not written yet.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, RECT};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_TEXTURE_CLEARTYPE_3x1, DWriteCreateFactory, IDWriteFactory2, IDWriteFontFace,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_GLYPH_RUN, DWRITE_GRID_FIT_MODE_DEFAULT,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
    DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
};

use crate::atlas::{GlyphId, Ink};
use crate::{Error, Result};

/// How many glyphs go into one `CreateGlyphRunAnalysis`.
///
/// Inside the 4–64 window `experiments/g4b-batched-raster` measured as flat at ~1.8x, and well
/// clear of the 256 mark where the larger intermediate bitmap makes the whole thing slower than
/// not batching at all.
const BATCH: usize = 16;

/// The uniform cell every glyph of one `(face, size)` is rasterised into, in device pixels
/// relative to the pen position on the baseline.
///
/// Uniform is the point: `SPEC.md` §3.2's atlas gives every glyph exactly one slot, which is what
/// makes eviction O(1) with no repacking and no fragmentation.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CellBox {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

impl CellBox {
    pub fn ink(self) -> Ink {
        Ink {
            width: self.width as u16,
            height: self.height as u16,
            left: self.left as i16,
            top: self.top as i16,
        }
    }

    fn bytes(self) -> usize {
        self.width as usize * self.height as usize * 4
    }
}

/// One rasterised glyph, ready to upload into an atlas slot.
///
/// The bytes are RGBA: **R, G and B are the three ClearType coverages** and **A is their average**,
/// so one sheet serves both subpixel and greyscale rendering at no extra memory (G4). The shader
/// premultiplies; see `SPEC.md` §3.2's dual-source blend.
pub struct Raster {
    pub ink: Ink,
    pub rgba: Vec<u8>,
}

/// A resolved font face. Glyph ids are face-local, which is why [`crate::atlas::GlyphKey`] carries
/// one.
pub struct Face {
    face: IDWriteFontFace,
    pub family: String,
}

impl Face {
    /// Bulk codepoint → glyph index. A codepoint the face does not have maps to **0**, the
    /// `.notdef` glyph, rather than failing — which is the signal to try the next face in the
    /// fallback chain (§3.3).
    pub fn glyph_indices(&self, codepoints: &[u32]) -> Vec<GlyphId> {
        let mut out = vec![0u16; codepoints.len()];
        unsafe {
            let _ = self.face.GetGlyphIndices(
                codepoints.as_ptr(),
                codepoints.len() as u32,
                out.as_mut_ptr(),
            );
        }
        out
    }
}

pub struct Rasteriser {
    factory: IDWriteFactory2,
}

/// Builds a glyph run over borrowed glyph ids.
///
/// **`DWRITE_GLYPH_RUN::fontFace` is a `ManuallyDrop<Option<IDWriteFontFace>>`.** Writing a
/// `transmute_copy` *in* is safe — nothing is released. Copying one *out* into an owned
/// `IDWriteFontFace` releases a reference that was never added, and the refcount underflow surfaces
/// later as a use-after-free (G4).
fn make_run(
    face: &IDWriteFontFace,
    glyphs: &[GlyphId],
    advances: &[f32],
    em: f32,
) -> DWRITE_GLYPH_RUN {
    DWRITE_GLYPH_RUN {
        fontFace: unsafe { std::mem::transmute_copy(face) },
        fontEmSize: em,
        glyphCount: glyphs.len() as u32,
        glyphIndices: glyphs.as_ptr(),
        glyphAdvances: advances.as_ptr(),
        glyphOffsets: std::ptr::null(),
        isSideways: BOOL(0),
        bidiLevel: 0,
    }
}

/// The smallest box containing every inked rectangle, or `None` if none of them has ink.
///
/// Separated from the DirectWrite calls because that is the only way to test it: **a glyph with no
/// ink must contribute nothing**, and on the faces this machine has, an empty `RECT{0,0,0,0}`
/// happens to sit inside the ink extent already — so including it changes nothing and no test over
/// real glyphs can tell the two behaviours apart. It matters on a face whose ink box does not
/// contain the origin, and the cell origin *is* every glyph's placement offset, so getting it wrong
/// shifts the whole grid.
fn tighten(bounds: &[RECT]) -> Option<CellBox> {
    let mut out: Option<CellBox> = None;
    for b in bounds {
        if b.right <= b.left || b.bottom <= b.top {
            continue;
        }
        out = Some(match out {
            None => CellBox {
                left: b.left,
                top: b.top,
                width: (b.right - b.left) as u32,
                height: (b.bottom - b.top) as u32,
            },
            Some(c) => {
                let left = c.left.min(b.left);
                let top = c.top.min(b.top);
                let right = (c.left + c.width as i32).max(b.right);
                let bottom = (c.top + c.height as i32).max(b.bottom);
                CellBox {
                    left,
                    top,
                    width: (right - left) as u32,
                    height: (bottom - top) as u32,
                }
            }
        });
    }
    out
}

impl Rasteriser {
    pub fn new() -> Result<Self> {
        Ok(Self {
            factory: unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? },
        })
    }

    /// Resolves the first installed family from a candidate list.
    ///
    /// A family that is not installed is **skipped, not substituted** — silently rendering a log
    /// in whatever the system picked instead is the sort of thing that goes unnoticed until a
    /// column of hex no longer lines up. §3.3's fallback chain is a list for this reason.
    pub fn resolve(&self, candidates: &[&str]) -> Result<Face> {
        let mut collection = None;
        unsafe {
            self.factory
                .GetSystemFontCollection(&mut collection, false)?
        };
        let collection = collection.ok_or_else(|| Error("no system font collection".to_owned()))?;

        for name in candidates {
            let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
            let mut index = 0u32;
            let mut exists = BOOL(0);
            unsafe {
                collection.FindFamilyName(PCWSTR(wide.as_ptr()), &mut index, &mut exists)?;
            }
            if !exists.as_bool() {
                continue;
            }
            let family = unsafe { collection.GetFontFamily(index)? };
            let font = unsafe {
                family.GetFirstMatchingFont(
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                )?
            };
            return Ok(Face {
                face: unsafe { font.CreateFontFace()? },
                family: (*name).to_owned(),
            });
        }
        Err(Error(format!("none of {candidates:?} is installed")))
    }

    /// The smallest uniform cell containing every one of these glyphs' ink.
    ///
    /// **Measure over a representative set** — for a log grid, the printable ASCII of the face at
    /// minimum. A glyph that turns up later and does not fit is refused by the atlas
    /// (`InsertError::TooLarge`) rather than clipped into its neighbour's cell, which is the
    /// failure this method exists to make loud.
    ///
    /// This costs one analysis per glyph, so it is the expensive way to rasterise and the cheap way
    /// to measure. It runs once per `(face, size)`, not per frame.
    pub fn measure_cell(&self, face: &Face, glyphs: &[GlyphId], px_per_em: u16) -> Result<CellBox> {
        tighten(&self.per_glyph_bounds(face, glyphs, px_per_em)?)
            .ok_or_else(|| Error("no glyph in the measured set has any ink at all".to_owned()))
    }

    fn per_glyph_bounds(
        &self,
        face: &Face,
        glyphs: &[GlyphId],
        px_per_em: u16,
    ) -> Result<Vec<RECT>> {
        let advance = [0.0f32];
        let mut out = Vec::with_capacity(glyphs.len());
        for glyph in glyphs {
            let one = [*glyph];
            let run = make_run(&face.face, &one, &advance, px_per_em as f32);
            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &run,
                    None,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_MEASURING_MODE_NATURAL,
                    DWRITE_GRID_FIT_MODE_DEFAULT,
                    DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                    0.0,
                    0.0,
                )?
            };
            out.push(unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)? });
        }
        Ok(out)
    }

    /// Rasterises glyphs into one cell each. `None` means the glyph has no ink — a space, or a
    /// codepoint the face lacks — which the atlas records as blank so it is never asked for again.
    pub fn rasterise(
        &self,
        face: &Face,
        glyphs: &[GlyphId],
        px_per_em: u16,
        cell: CellBox,
    ) -> Result<Vec<Option<Raster>>> {
        self.rasterise_in_batches(face, glyphs, px_per_em, cell, BATCH)
    }

    fn rasterise_in_batches(
        &self,
        face: &Face,
        glyphs: &[GlyphId],
        px_per_em: u16,
        cell: CellBox,
        batch: usize,
    ) -> Result<Vec<Option<Raster>>> {
        let mut out = Vec::with_capacity(glyphs.len());
        if glyphs.is_empty() {
            return Ok(out);
        }
        let batch = batch.max(1);
        // No inter-glyph padding: G4b measured batched cells bit-identical to per-glyph cells at a
        // gap of zero, so DirectWrite does not let the ClearType filter bleed across the step.
        let step = cell.width as i32;
        let advances = vec![step as f32; batch.min(glyphs.len())];
        let mut strip: Vec<u8> = Vec::new();

        for chunk in glyphs.chunks(batch) {
            let run = make_run(&face.face, chunk, &advances, px_per_em as f32);
            let analysis = unsafe {
                self.factory.CreateGlyphRunAnalysis(
                    &run,
                    None,
                    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                    DWRITE_MEASURING_MODE_NATURAL,
                    DWRITE_GRID_FIT_MODE_DEFAULT,
                    DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                    0.0,
                    0.0,
                )?
            };
            let bounds: RECT =
                unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)? };
            let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
            if w <= 0 || h <= 0 {
                // The whole strip is empty, so every glyph in it is.
                out.extend(chunk.iter().map(|_| None));
                continue;
            }

            strip.clear();
            strip.resize((w * h * 3) as usize, 0);
            unsafe {
                analysis.CreateAlphaTexture(DWRITE_TEXTURE_CLEARTYPE_3x1, &bounds, &mut strip)?
            };

            for i in 0..chunk.len() {
                out.push(self.cut_cell(&strip, &bounds, w, i as i32 * step, cell));
            }
        }
        Ok(out)
    }

    /// Copies one glyph's cell out of the strip, converting the three ClearType coverages to RGBA
    /// with the average in alpha.
    fn cut_cell(
        &self,
        strip: &[u8],
        bounds: &RECT,
        strip_w: i32,
        pen_x: i32,
        cell: CellBox,
    ) -> Option<Raster> {
        let mut rgba = vec![0u8; cell.bytes()];
        let mut inked = false;
        let origin_x = pen_x + cell.left;

        for y in 0..cell.height as i32 {
            let ay = cell.top + y;
            if ay < bounds.top || ay >= bounds.bottom {
                continue;
            }
            let src_row = ((ay - bounds.top) * strip_w) as usize;
            for x in 0..cell.width as i32 {
                let ax = origin_x + x;
                if ax < bounds.left || ax >= bounds.right {
                    continue;
                }
                let s = (src_row + (ax - bounds.left) as usize) * 3;
                let (r, g, b) = (strip[s], strip[s + 1], strip[s + 2]);
                if r | g | b != 0 {
                    inked = true;
                }
                let d = (y as usize * cell.width as usize + x as usize) * 4;
                rgba[d] = r;
                rgba[d + 1] = g;
                rgba[d + 2] = b;
                rgba[d + 3] = ((r as u16 + g as u16 + b as u16) / 3) as u8;
            }
        }

        // A cell of nothing is a blank, not a raster. Uploading it would spend a slot on a space
        // and re-request it on every frame — G4's 440-spurious-misses-per-frame bug.
        inked.then_some(Raster {
            ink: cell.ink(),
            rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Faces that ship with Windows, most monospace-appropriate first. Consolas has been in the
    /// box since Vista, so a machine with none of these is unusual enough to be worth saying out
    /// loud rather than passing quietly.
    const CANDIDATES: &[&str] = &["Cascadia Mono", "Consolas", "Courier New", "Segoe UI"];

    const EM: u16 = 14;

    fn ascii_glyphs(face: &Face) -> Vec<GlyphId> {
        let codepoints: Vec<u32> = (0x21..0x7Fu32).collect();
        face.glyph_indices(&codepoints)
    }

    fn face_or_skip(what: &str) -> Option<(Rasteriser, Face)> {
        let rasteriser = match Rasteriser::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED {what}: no DirectWrite factory ({e})");
                return None;
            }
        };
        match rasteriser.resolve(CANDIDATES) {
            Ok(face) => Some((rasteriser, face)),
            Err(e) => {
                eprintln!("SKIPPED {what}: no usable font ({e})");
                None
            }
        }
    }

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT {
            left,
            top,
            right,
            bottom,
        }
    }

    /// The extent arithmetic, on synthetic boxes that deliberately do *not* contain the origin —
    /// which is the only way to tell "ignores empty rectangles" from "happens to agree with them".
    #[test]
    fn an_empty_rectangle_does_not_move_the_cell() {
        let inked = [rect(4, -12, 12, -2), rect(6, -10, 14, 3)];
        let tight = tighten(&inked).expect("two inked boxes");
        assert_eq!(
            tight,
            CellBox {
                left: 4,
                top: -12,
                width: 10,
                height: 15
            }
        );

        let with_blanks = [
            rect(0, 0, 0, 0),
            inked[0],
            rect(0, 0, 0, 0),
            inked[1],
            rect(7, 5, 7, 9),
        ];
        assert_eq!(
            tighten(&with_blanks),
            Some(tight),
            "a glyph with no ink dragged the cell towards the origin"
        );
    }

    #[test]
    fn a_set_with_no_ink_at_all_has_no_cell() {
        assert_eq!(tighten(&[]), None);
        assert_eq!(tighten(&[rect(0, 0, 0, 0), rect(3, 4, 3, 9)]), None);
    }

    #[test]
    fn a_family_that_is_not_installed_is_reported_rather_than_substituted() {
        let Some((rasteriser, _)) =
            face_or_skip("a_family_that_is_not_installed_is_reported_rather_than_substituted")
        else {
            return;
        };
        assert!(
            rasteriser
                .resolve(&["Tailhawk No Such Family 90210"])
                .is_err(),
            "an absent family must not quietly resolve to whatever the system picked"
        );
    }

    #[test]
    fn a_codepoint_the_face_lacks_maps_to_notdef() {
        let Some((_, face)) = face_or_skip("a_codepoint_the_face_lacks_maps_to_notdef") else {
            return;
        };
        let [a, private_use] = face.glyph_indices(&[b'A' as u32, 0xF8FF])[..] else {
            unreachable!("two codepoints in, two glyph ids out")
        };
        assert_ne!(a, 0, "'A' should exist in a text face");
        assert_eq!(
            private_use, 0,
            "an unmapped codepoint is .notdef, which is what makes fallback possible"
        );
    }

    /// The rule from G4b's clipping accident: a cell derived from measured bounds must actually
    /// contain what it was measured from.
    #[test]
    fn the_cell_contains_the_ink_of_every_glyph_it_was_measured_from() {
        let Some((rasteriser, face)) =
            face_or_skip("the_cell_contains_the_ink_of_every_glyph_it_was_measured_from")
        else {
            return;
        };
        let glyphs = ascii_glyphs(&face);
        let cell = rasteriser
            .measure_cell(&face, &glyphs, EM)
            .expect("measure");

        for (glyph, b) in glyphs
            .iter()
            .zip(rasteriser.per_glyph_bounds(&face, &glyphs, EM).unwrap())
        {
            if b.right <= b.left || b.bottom <= b.top {
                continue;
            }
            assert!(
                b.left >= cell.left
                    && b.right <= cell.left + cell.width as i32
                    && b.top >= cell.top
                    && b.bottom <= cell.top + cell.height as i32,
                "glyph {glyph} at {b:?} escapes the cell {cell:?}"
            );
        }
    }

    /// Containment alone would still pass if the cell were inflated — and a cell inflated at the
    /// left or top shifts every glyph in the grid, because the cell origin *is* the placement
    /// offset. So the cell must also be tight: something has to touch each edge.
    ///
    /// The concrete way to get this wrong is to let a glyph with no ink contribute its empty
    /// `RECT{0,0,0,0}` to the extent, which drags the box towards the origin.
    #[test]
    fn the_measured_cell_is_tight_on_all_four_sides() {
        let Some((rasteriser, face)) = face_or_skip("the_measured_cell_is_tight_on_all_four_sides")
        else {
            return;
        };
        let mut glyphs = ascii_glyphs(&face);
        glyphs.extend(face.glyph_indices(&[b' ' as u32, 0xF8FF]));
        let cell = rasteriser
            .measure_cell(&face, &glyphs, EM)
            .expect("measure");
        let bounds = rasteriser.per_glyph_bounds(&face, &glyphs, EM).unwrap();
        let inked: Vec<&RECT> = bounds
            .iter()
            .filter(|b| b.right > b.left && b.bottom > b.top)
            .collect();

        assert!(
            inked.iter().any(|b| b.left == cell.left),
            "nothing reaches the cell's left edge: {cell:?}"
        );
        assert!(
            inked.iter().any(|b| b.top == cell.top),
            "nothing reaches the cell's top edge: {cell:?}"
        );
        assert!(
            inked
                .iter()
                .any(|b| b.right == cell.left + cell.width as i32),
            "nothing reaches the cell's right edge: {cell:?}"
        );
        assert!(
            inked
                .iter()
                .any(|b| b.bottom == cell.top + cell.height as i32),
            "nothing reaches the cell's bottom edge: {cell:?}"
        );
    }

    /// **G4b's headline finding, kept as a regression test in the product code.** Grid fitting and
    /// the ClearType filter are both position-dependent, so a glyph rasterised amid neighbours
    /// could legitimately differ from the same glyph rasterised alone. It does not — and if a
    /// future DirectWrite ever makes it so, batching has to go, and this is what says so.
    #[test]
    fn batched_rasterisation_is_bit_identical_to_one_glyph_at_a_time() {
        let Some((rasteriser, face)) =
            face_or_skip("batched_rasterisation_is_bit_identical_to_one_glyph_at_a_time")
        else {
            return;
        };
        let glyphs = ascii_glyphs(&face);
        let cell = rasteriser
            .measure_cell(&face, &glyphs, EM)
            .expect("measure");

        let alone = rasteriser
            .rasterise_in_batches(&face, &glyphs, EM, cell, 1)
            .expect("per-glyph");
        for batch in [4, 16, 64] {
            let batched = rasteriser
                .rasterise_in_batches(&face, &glyphs, EM, cell, batch)
                .expect("batched");
            assert_eq!(batched.len(), alone.len());
            for (i, (a, b)) in alone.iter().zip(&batched).enumerate() {
                match (a, b) {
                    (None, None) => {}
                    (Some(a), Some(b)) => assert!(
                        a.rgba == b.rgba,
                        "glyph {} (index {i}) differs at batch {batch}",
                        glyphs[i]
                    ),
                    _ => panic!("glyph {} (index {i}) is blank in only one arm", glyphs[i]),
                }
            }
        }
    }

    #[test]
    fn a_space_has_no_ink_and_a_letter_does() {
        let Some((rasteriser, face)) = face_or_skip("a_space_has_no_ink_and_a_letter_does") else {
            return;
        };
        let cell = rasteriser
            .measure_cell(&face, &ascii_glyphs(&face), EM)
            .expect("measure");
        let glyphs = face.glyph_indices(&[b' ' as u32, b'A' as u32]);
        let rasters = rasteriser
            .rasterise(&face, &glyphs, EM, cell)
            .expect("raster");

        assert!(
            rasters[0].is_none(),
            "a space must be recorded as blank, or every space is a miss on every frame"
        );
        let a = rasters[1].as_ref().expect("'A' has ink");
        assert_eq!(a.ink, cell.ink());
        assert_eq!(
            a.rgba.len(),
            cell.width as usize * cell.height as usize * 4,
            "the atlas is promised exactly one cell's worth of bytes"
        );
        assert!(
            a.rgba.chunks(4).any(|px| px[3] > 0),
            "alpha carries the average of the three coverages and cannot be all zero on an inked glyph"
        );
    }
}
