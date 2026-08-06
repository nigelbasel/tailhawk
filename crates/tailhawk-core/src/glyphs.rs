//! The join: glyph id in, quad out.
//!
//! [`crate::atlas`] owns the slots, [`crate::raster`] makes the pixels, [`crate::sheet`] holds them
//! and [`crate::text`] draws them. This is the piece that puts those four together and enforces the
//! one rule that spans all of them — `SPEC.md` §3.2's **a frame must never block on rasterisation.**
//!
//! So [`GlyphCache::quad`] never rasterises anything. A glyph that is not resident yet returns a
//! **placeholder** quad and is queued; [`GlyphCache::flush_misses`] does the rasterising, and the
//! caller runs it **after presenting**, not before drawing. That ordering *is* the requirement: a
//! genuinely cold 1,500-glyph viewport measures 162 ms — 8 to 10 frames — the first time a machine
//! ever renders those glyphs, because DirectWrite's cross-process font cache is empty for them
//! (`experiments/g4b-batched-raster`). Steady state is the same viewport in 4.4 ms, inside one
//! frame, so this is a first-run-experience requirement rather than a permanent tax.

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::atlas::{Atlas, GlyphId, GlyphKey, Residency, Synthetic};
use crate::raster::{CellBox, Face, Rasteriser};
use crate::sheet::Sheet;
use crate::text::{Instance, MODE_MONO_SUBPIXEL};
use crate::{Error, Result};

/// One sheet, square, 4 bytes per texel — 4 MB. G4 used this size, and against `SPEC.md` §11.2's
/// 120 MB whole-process claim it is not the term that matters.
const SHEET: u16 = 1024;

/// What a glyph the atlas has never seen draws until its raster arrives: a hollow box, one texel
/// thick, at the coverage below.
///
/// A hollow box rather than a solid block because a screen briefly full of solid blocks reads as
/// corruption, while a screen of outlines reads as loading — and it is what a terminal shows for a
/// glyph it cannot render, so it is already a familiar shape.
const PLACEHOLDER_COVERAGE: u8 = 90;

pub struct GlyphCache {
    rasteriser: Rasteriser,
    face: Face,
    atlas: Atlas,
    sheet: Sheet,
    px_per_em: u16,
    cell: CellBox,
    /// Where the placeholder lives, in sheet texels. Outside every slot the atlas can hand out, so
    /// it can never be evicted.
    placeholder: (u16, u16),
    misses: Vec<GlyphKey>,
}

impl GlyphCache {
    /// Resolves a face, measures its cell, and lays a sheet out around that cell.
    ///
    /// The measurement set is the printable ASCII of the face, which is what a log line is mostly
    /// made of. A glyph that turns up later and does not fit its slot is refused by the atlas rather
    /// than clipped into its neighbour — see [`crate::atlas::InsertError::TooLarge`].
    pub fn new(device: &ID3D11Device, candidates: &[&str], px_per_em: u16) -> Result<Self> {
        let rasteriser = Rasteriser::new()?;
        let face = rasteriser.resolve(candidates)?;
        let measured: Vec<GlyphId> = face.glyph_indices(&(0x21..0x7Fu32).collect::<Vec<_>>());
        let cell = rasteriser.measure_cell(&face, &measured, px_per_em)?;

        let (cell_w, cell_h) = (cell.width as u16, cell.height as u16);
        if cell_w == 0 || cell_h > SHEET / 2 || cell_w > SHEET {
            return Err(Error(format!(
                "a {cell_w}x{cell_h} cell does not lay out sensibly on a {SHEET}x{SHEET} sheet"
            )));
        }
        // The last whole row of cells is reserved for the placeholder, so the atlas is told the
        // sheet is one row shorter than it is. That is what makes the placeholder un-evictable
        // without the atlas needing to know it exists.
        let rows = SHEET / cell_h;
        let atlas_height = (rows - 1) * cell_h;

        let sheet = Sheet::mono(device, SHEET, SHEET)?;
        Ok(Self {
            rasteriser,
            face,
            atlas: Atlas::new(SHEET, atlas_height, cell_w, cell_h),
            sheet,
            px_per_em,
            cell,
            placeholder: (0, atlas_height),
            misses: Vec::new(),
        })
    }

    pub fn cell(&self) -> CellBox {
        self.cell
    }

    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    pub fn face(&self) -> &Face {
        &self.face
    }

    pub fn capacity(&self) -> usize {
        self.atlas.capacity()
    }

    /// Uploads the placeholder. Separate from `new` because it needs a context, and because it is
    /// the one thing in the sheet that has to be re-uploaded after a device is rebuilt.
    pub fn prime(&mut self, context: &ID3D11DeviceContext) -> bool {
        let (w, h) = (self.cell.width as usize, self.cell.height as usize);
        let mut rgba = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                if y == 0 || y == h - 1 || x == 0 || x == w - 1 {
                    let i = (y * w + x) * 4;
                    rgba[i..i + 4].fill(PLACEHOLDER_COVERAGE);
                }
            }
        }
        self.sheet.upload(
            context,
            self.placeholder.0,
            self.placeholder.1,
            self.cell.width as u16,
            self.cell.height as u16,
            &rgba,
        )
    }

    pub fn key(&self, glyph: GlyphId) -> GlyphKey {
        GlyphKey {
            face: 0,
            glyph,
            px_per_em: self.px_per_em,
            synthetic: Synthetic::default(),
        }
    }

    /// Starts a frame. Everything drawn since the last call becomes evictable again, and the miss
    /// queue is emptied — a glyph that has scrolled off is not worth rasterising.
    pub fn begin_frame(&mut self) {
        self.atlas.begin_frame();
        self.misses.clear();
    }

    /// The quad to draw a glyph with, at a pen position on the baseline.
    ///
    /// `None` means the glyph has no ink and nothing should be drawn. **This never rasterises**; a
    /// glyph that is not resident yet gets the placeholder and joins the miss queue.
    pub fn quad(
        &mut self,
        glyph: GlyphId,
        pen_x: f32,
        baseline_y: f32,
        tint: [f32; 4],
    ) -> Option<Instance> {
        let key = self.key(glyph);
        let (x, y) = (
            pen_x + self.cell.left as f32,
            baseline_y + self.cell.top as f32,
        );
        let size = [self.cell.width as f32, self.cell.height as f32];

        let (uv0, uv1) = match self.atlas.lookup(&key) {
            Residency::Blank => return None,
            Residency::Resident(p) => (
                [p.x as f32, p.y as f32],
                [(p.x + p.ink.width) as f32, (p.y + p.ink.height) as f32],
            ),
            Residency::Absent => {
                if !self.misses.contains(&key) {
                    self.misses.push(key);
                }
                let (px, py) = self.placeholder;
                (
                    [px as f32, py as f32],
                    [
                        (px + self.cell.width as u16) as f32,
                        (py + self.cell.height as u16) as f32,
                    ],
                )
            }
        };

        Some(Instance {
            pos: [x, y],
            size,
            uv0,
            uv1,
            tint,
            mode: MODE_MONO_SUBPIXEL,
            pad: [0; 3],
        })
    }

    pub fn pending(&self) -> usize {
        self.misses.len()
    }

    /// Rasterises everything queued this frame and uploads it. **Call this after presenting** —
    /// that is what keeps rasterisation off the paint path.
    ///
    /// Returns how many glyphs became resident. A glyph that could not be taken in is simply not
    /// resident yet: it draws a placeholder again next frame and is queued again, which is the
    /// correct behaviour for a sheet that is momentarily full.
    pub fn flush_misses(&mut self, context: &ID3D11DeviceContext) -> Result<usize> {
        if self.misses.is_empty() {
            return Ok(0);
        }
        let queued = std::mem::take(&mut self.misses);
        let glyphs: Vec<GlyphId> = queued.iter().map(|k| k.glyph).collect();
        let rasters = self
            .rasteriser
            .rasterise(&self.face, &glyphs, self.px_per_em, self.cell)?;

        let mut landed = 0;
        for (key, raster) in queued.into_iter().zip(rasters) {
            let Some(raster) = raster else {
                // No ink. Recorded so it is never asked for again — without this, every space in
                // the viewport is a miss on every frame.
                self.atlas.insert_blank(key);
                continue;
            };
            let Ok(placement) = self.atlas.insert(key, raster.ink) else {
                continue;
            };
            if self.sheet.upload(
                context,
                placement.x,
                placement.y,
                raster.ink.width,
                raster.ink.height,
                &raster.rgba,
            ) {
                landed += 1;
            }
        }
        Ok(landed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::offscreen::Offscreen;
    use crate::text::TextPipeline;

    const CANDIDATES: &[&str] = &["Cascadia Mono", "Consolas", "Courier New", "Segoe UI"];
    const EM: u16 = 14;
    const TARGET: u32 = 64;
    const INK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const PAPER: [f32; 4] = [0.85, 0.85, 0.85, 1.0];

    /// One code point, one glyph. **This is not shaping** — it has no ligatures, no mark
    /// attachment and no reordering, so it is wrong for Devanagari, Arabic and anything else
    /// `SPEC.md` §3.3 cares about. It is confined to this test module deliberately: the grid must
    /// not be built on it.
    fn glyphs_for(cache: &GlyphCache, text: &str) -> Vec<GlyphId> {
        let codepoints: Vec<u32> = text.chars().map(|c| c as u32).collect();
        cache.face().glyph_indices(&codepoints)
    }

    fn cache_or_skip(what: &str) -> Option<(Offscreen, GlyphCache)> {
        let off = match Offscreen::new(TARGET, TARGET) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("SKIPPED {what}: no D3D11 device ({e})");
                return None;
            }
        };
        match GlyphCache::new(off.device(), CANDIDATES, EM) {
            Ok(mut cache) => {
                assert!(cache.prime(off.context()), "the placeholder must upload");
                Some((off, cache))
            }
            Err(e) => {
                eprintln!("SKIPPED {what}: no usable font ({e})");
                None
            }
        }
    }

    /// `SPEC.md` §3.2's requirement, stated as a test: asking for a glyph never rasterises it.
    #[test]
    fn asking_for_a_glyph_never_rasterises_it() {
        let Some((off, mut cache)) = cache_or_skip("asking_for_a_glyph_never_rasterises_it") else {
            return;
        };
        let glyph = glyphs_for(&cache, "A")[0];

        cache.begin_frame();
        let placeholder = cache.quad(glyph, 4.0, 20.0, INK).expect("a quad to draw");
        assert_eq!(cache.pending(), 1, "the glyph should have been queued");

        // Only now, and only because the caller asked.
        assert_eq!(cache.flush_misses(off.context()).expect("flush"), 1);
        assert_eq!(cache.pending(), 0);

        cache.begin_frame();
        let real = cache.quad(glyph, 4.0, 20.0, INK).expect("a quad to draw");
        assert_eq!(cache.pending(), 0, "a resident glyph is not a miss");
        assert_ne!(
            placeholder.uv0, real.uv0,
            "the second frame should sample the glyph's own slot, not the placeholder's"
        );
        assert_eq!(
            placeholder.pos, real.pos,
            "a placeholder must occupy exactly the cell the real glyph will"
        );
    }

    /// The placeholder is outside every slot the atlas can hand out, so no amount of eviction can
    /// take it — which is what stops a full sheet from having nothing to draw a miss with.
    #[test]
    fn the_placeholder_cannot_be_evicted() {
        let Some((off, mut cache)) = cache_or_skip("the_placeholder_cannot_be_evicted") else {
            return;
        };
        let (px, py) = cache.placeholder;
        let cell = cache.cell();
        assert!(
            py + cell.height as u16 <= SHEET,
            "the reserved cell must be inside the sheet"
        );

        // Fill the atlas past capacity, one frame per glyph so eviction is allowed.
        let capacity = cache.capacity();
        for g in 0..capacity + 32 {
            cache.begin_frame();
            cache.quad(g as GlyphId, 0.0, 20.0, INK);
            cache.flush_misses(off.context()).expect("flush");
        }

        cache.begin_frame();
        let miss = cache
            .quad(u16::MAX - 1, 0.0, 20.0, INK)
            .expect("an unseen glyph still draws");
        assert_eq!(
            miss.uv0,
            [px as f32, py as f32],
            "a miss must still have the placeholder to draw"
        );
    }

    #[test]
    fn a_space_is_recorded_as_blank_and_stops_being_a_miss() {
        let Some((off, mut cache)) =
            cache_or_skip("a_space_is_recorded_as_blank_and_stops_being_a_miss")
        else {
            return;
        };
        let space = glyphs_for(&cache, " ")[0];

        cache.begin_frame();
        assert!(
            cache.quad(space, 4.0, 20.0, INK).is_some(),
            "first sight is a placeholder"
        );
        assert_eq!(
            cache.flush_misses(off.context()).expect("flush"),
            0,
            "a space has no raster to upload"
        );

        cache.begin_frame();
        assert!(
            cache.quad(space, 4.0, 20.0, INK).is_none(),
            "a known-blank glyph draws nothing at all"
        );
        assert_eq!(cache.pending(), 0, "and is never queued again");
    }

    #[test]
    fn the_same_glyph_twice_in_one_frame_is_queued_once() {
        let Some((_off, mut cache)) =
            cache_or_skip("the_same_glyph_twice_in_one_frame_is_queued_once")
        else {
            return;
        };
        let glyph = glyphs_for(&cache, "x")[0];
        cache.begin_frame();
        for _ in 0..5 {
            cache.quad(glyph, 0.0, 20.0, INK);
        }
        assert_eq!(cache.pending(), 1);
    }

    /// A glyph queued on a frame that has scrolled away is not worth rasterising.
    #[test]
    fn a_new_frame_forgets_the_previous_frames_misses() {
        let Some((_off, mut cache)) =
            cache_or_skip("a_new_frame_forgets_the_previous_frames_misses")
        else {
            return;
        };
        cache.begin_frame();
        cache.quad(glyphs_for(&cache, "q")[0], 0.0, 20.0, INK);
        assert_eq!(cache.pending(), 1);
        cache.begin_frame();
        assert_eq!(cache.pending(), 0);
    }

    /// End to end, on real pixels: text goes in, ink comes out where it was put — and the frame
    /// before the flush is *not* blank either, because the placeholder drew.
    #[test]
    fn real_text_reaches_real_pixels() {
        let Some((off, mut cache)) = cache_or_skip("real_text_reaches_real_pixels") else {
            return;
        };
        let pipeline = TextPipeline::new(off.device()).expect("pipeline");
        let glyphs = glyphs_for(&cache, "Hi");
        let baseline = 20.0;
        let advance = cache.cell().width as f32;

        let draw = |cache: &mut GlyphCache| -> Vec<Instance> {
            cache.begin_frame();
            glyphs
                .iter()
                .enumerate()
                .filter_map(|(i, g)| cache.quad(*g, 4.0 + i as f32 * advance, baseline, INK))
                .collect()
        };

        // Frame one: nothing is resident, so this is the placeholder path.
        let first = draw(&mut cache);
        assert_eq!(first.len(), glyphs.len());
        off.clear(PAPER);
        pipeline
            .draw(off.context(), cache.sheet(), (TARGET, TARGET), &first)
            .expect("draw");
        let placeholders = off.read_back().expect("read back");
        assert!(
            darkest(&placeholders) < 200,
            "the placeholder frame should not be blank paper"
        );

        // Then the rasterisation the frame deliberately did not wait for.
        assert_eq!(cache.flush_misses(off.context()).expect("flush"), 2);

        let second = draw(&mut cache);
        off.clear(PAPER);
        pipeline
            .draw(off.context(), cache.sheet(), (TARGET, TARGET), &second)
            .expect("draw");
        let real = off.read_back().expect("read back");
        assert!(
            darkest(&real) < 100,
            "black text on light paper should put something genuinely dark on the target"
        );
        assert!(
            all_paper(&real, 30),
            "nothing should have been drawn outside the two cells"
        );
    }

    /// The darkest single channel anywhere in the cells the text occupies.
    fn darkest(pixels: &crate::gpu::offscreen::Pixels) -> u8 {
        let mut min = 255;
        for y in 0..40u32 {
            for x in 0..40u32 {
                min = min.min(*pixels.at(x, y)[..3].iter().min().expect("three channels"));
            }
        }
        min
    }

    /// True when every pixel below `y` is still untouched paper — the text sits on one baseline, so
    /// anything down there came from a quad that escaped its cell.
    fn all_paper(pixels: &crate::gpu::offscreen::Pixels, y: u32) -> bool {
        (y..TARGET)
            .all(|row| (0..TARGET).all(|col| pixels.at(col, row)[..3].iter().all(|&c| c > 200)))
    }
}
