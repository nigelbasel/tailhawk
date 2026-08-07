//! The text pass — a viewport of rows becomes a list of quads.
//!
//! This is the join `SPEC.md` §3.2 and §3.3 describe from opposite ends. [`crate::view`] says which
//! rows and columns are on screen; [`crate::shape`] says which glyphs draw a line;
//! [`crate::glyphs`] turns a glyph into a quad without ever rasterising one; [`crate::text`] draws
//! them in a single instanced call. Nothing here is new work — it is the ordering that makes those
//! four correct together, and the ordering is where the requirements live.
//!
//! ## The pen is cell-driven, and that is §3.3 rather than a simplification
//!
//! A cluster is placed at [`HGrid::x_of_column`](crate::hgrid::HGrid::x_of_column) — **not** at a
//! pen advanced by the shaper's own advances. §3.3: "when a fallback font's advance width disagrees
//! with the primary, the cell grid wins and the glyph is centred within its cell." Advancing by the
//! shaped advances instead means a line containing one fallback glyph drifts out of column for the
//! rest of its length, and every row drifts by a different amount — which is exactly the misaligned
//! grid §3.3 exists to prevent. The shaper's offsets are still honoured, but only *within* a
//! cluster, where they position an attached mark relative to its base.
//!
//! ## Rasterisation is not on this path
//!
//! [`GlyphCache::quad`](crate::glyphs::GlyphCache::quad) returns a placeholder for a glyph that is
//! not resident and queues it. §3.2 requires that, and `experiments/g4b-batched-raster` measured
//! why: a genuinely cold 1,500-glyph viewport costs 162 ms — 8 to 10 frames — the first time a
//! machine renders those glyphs. [`Painter::flush_misses`] is what pays that cost, and the caller
//! runs it **after presenting**.
//!
//! **⚠ One consequence of that is visible and was not anticipated anywhere: a cold viewport draws a
//! placeholder box in every space.** Blankness is a property the atlas can only record *after*
//! rasterising a glyph, so on the first frame a space is `Absent` like any other glyph and gets the
//! hollow box; from the second it is `Blank` and draws nothing. A log line is 15–20% spaces, so the
//! first frame on a machine that has never rendered this face is a screen of text with boxes
//! between the words, for the 8–10 frames `experiments/g4b-batched-raster` measured. It self-heals
//! and it is not a correctness fault — `a_viewport_of_rows_reaches_real_pixels` asserts the warm
//! frame draws strictly *fewer* quads for exactly this reason — but "reads as loading" was the
//! placeholder's justification, and boxes between words reads as broken. Suppressing the
//! placeholder for whitespace clusters is a `glyphs.rs` change and is not made here.
//!
//! ## ⚠ Right-to-left rows are not placed yet, and this module says so out loud
//!
//! [`Shaped::visual_glyphs`](crate::shape::Shaped::visual_glyphs) orders the glyphs *inside* a run,
//! and `bidi.rs` implements UAX #9 rule L2 for that. **Which column an RTL run's clusters occupy is
//! a different question**, and [`crate::cell`] answers it in logical order — so an Arabic or Hebrew
//! row laid out here has its glyphs in the right shapes and the wrong columns.
//!
//! That is a gap, not a bug to be quietly tolerated, and the difference matters: a viewer that draws
//! Arabic backwards while looking finished is worse than one that admits it. So [`Laid::rtl_runs`]
//! counts the runs this pass could not place, every caller can see it, and
//! `an_arabic_row_reports_that_its_runs_are_not_placed` fails the day someone believes the problem
//! is solved. Doing it properly means deciding whether a *column* is a logical or a visual position
//! — which reaches into `selection.rs` and `cell.rs`, not just here.

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::glyphs::GlyphCache;
use crate::shape::Shaper;
use crate::text::{Instance, TextPipeline};
use crate::view::View;
use crate::Result;

/// What one row's layout produced, beyond the quads themselves.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Laid {
    /// Quads appended for this row.
    pub quads: usize,
    /// Glyphs that were not resident and are now queued for [`Painter::flush_misses`]. Non-zero on
    /// a cold viewport and zero in steady state; it is the number §3.2's placeholder rule exists
    /// for, and it must never be a reason to delay the frame.
    pub queued: usize,
    /// **Right-to-left runs this pass placed in logical columns, which is the wrong place.** See
    /// the module note. Zero for almost every line in almost every log; non-zero is a correctness
    /// claim this module is not yet entitled to make.
    pub rtl_runs: usize,
}

impl Laid {
    fn merge(&mut self, other: Laid) {
        self.quads += other.quads;
        self.queued += other.queued;
        self.rtl_runs += other.rtl_runs;
    }
}

/// The device-owned half of the grid: shaper, glyph cache and pipeline, plus the frame's quads.
///
/// **Everything here is bound to one D3D11 device.** The cache holds a [`Sheet`](crate::sheet), a
/// texture the device owns, so a painter must not outlive the device that built it — `gpu.rs`
/// rebuilds a lost device and bumps its generation, and drawing a sheet from the previous one
/// silently produces nothing. The owner is responsible for rebuilding this alongside the device;
/// `SPEC.md` §3.2 forbids making it a panic.
pub struct Painter {
    shaper: Shaper,
    cache: GlyphCache,
    pipeline: TextPipeline,
    px_per_em: u16,
    instances: Vec<Instance>,
}

impl Painter {
    pub fn new(device: &ID3D11Device, candidates: &[&str], px_per_em: u16) -> Result<Self> {
        Ok(Self {
            shaper: Shaper::new()?,
            cache: GlyphCache::new(device, candidates, px_per_em)?,
            pipeline: TextPipeline::new(device)?,
            px_per_em,
            instances: Vec::new(),
        })
    }

    /// Uploads the placeholder. Until this succeeds a missing glyph draws nothing rather than a box.
    pub fn prime(&mut self, context: &ID3D11DeviceContext) -> bool {
        self.cache.prime(context)
    }

    /// The measured cell — **what [`View`]'s metrics must be set from.** §3.1 requires integer cell
    /// advances derived from the face at the current scale, so these are the only honest source for
    /// `View::set_metrics`, and a constant would drift from the font on any DPI change.
    pub fn cell_width(&self) -> f32 {
        self.cache.cell().width as f32
    }

    pub fn row_height(&self) -> f32 {
        self.cache.cell().height as f32
    }

    /// Starts a frame: drops the previous frame's quads and its miss list.
    pub fn begin_frame(&mut self) {
        self.instances.clear();
        self.cache.begin_frame();
    }

    /// Lays out every visible row. `line_at` returns a row's text, or `None` for a row whose bytes
    /// are not in memory yet — which draws nothing rather than blocking, per §11.3.
    pub fn lay_out(
        &mut self,
        view: &View,
        tint: [f32; 4],
        mut line_at: impl FnMut(u64) -> Option<String>,
    ) -> Result<Laid> {
        let mut total = Laid::default();
        let rows: Vec<(u64, f32)> = view.grid().visible().map(|p| (p.row, p.y)).collect();
        for (row, y) in rows {
            let Some(line) = line_at(row) else {
                continue;
            };
            total.merge(self.lay_out_row(view, &line, y, tint)?);
        }
        Ok(total)
    }

    /// One row. `y` is the row's top edge, viewport-relative, from [`crate::grid::PlacedRow`].
    pub fn lay_out_row(&mut self, view: &View, line: &str, y: f32, tint: [f32; 4]) -> Result<Laid> {
        let slice = view.slice(line);
        if slice.bytes.is_empty() {
            return Ok(Laid::default());
        }
        let text = &line[slice.bytes.clone()];
        let shaped = self.shaper.shape(self.cache.face(), text, self.px_per_em)?;

        // The cell box's `top` is the ink's offset from the baseline and is negative above it, and
        // `quad` adds it back. Subtracting it here therefore lands the cell exactly on the row's top
        // edge — the one place screen space and font metrics meet, resolved once.
        let baseline_y = y - self.cache.cell().top as f32;

        let mut laid = Laid {
            rtl_runs: shaped.runs.iter().filter(|r| r.level % 2 == 1).count(),
            ..Laid::default()
        };

        for cluster in &shaped.clusters {
            if cluster.glyph_count == 0 {
                // Absorbed into a preceding ligature. A real state, not a failure: the glyph that
                // draws this cluster was already emitted against the cluster it was reported on.
                continue;
            }
            // The cluster's byte offset is relative to the slice; the column is a property of the
            // whole line, so the slice's own start has to go back on before asking.
            let column = view
                .cells()
                .cell_at_byte(line, slice.bytes.start + cluster.span.byte);
            let cluster_x = view.hgrid().x_of_column(column);

            let mut pen = cluster_x;
            for i in cluster.first_glyph..cluster.first_glyph + cluster.glyph_count {
                let offset = shaped.offsets[i];
                let before = self.cache.pending();
                if let Some(quad) = self.cache.quad(
                    shaped.glyphs[i],
                    pen + offset.advance,
                    baseline_y - offset.ascender,
                    tint,
                ) {
                    self.instances.push(quad);
                    laid.quads += 1;
                }
                laid.queued += self.cache.pending() - before;
                // Within the cluster only. The next *cluster* starts at its own column, never here
                // — see the module note on why the cell grid wins.
                pen += shaped.advances[i];
            }
        }
        Ok(laid)
    }

    /// The frame's quads, for a caller that wants to inspect or extend them before drawing.
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// Draws the frame. The render target and viewport must already be set.
    pub fn draw(&self, context: &ID3D11DeviceContext, viewport: (u32, u32)) -> Result<()> {
        self.pipeline
            .draw(context, self.cache.sheet(), viewport, &self.instances)
    }

    /// Rasterises what this frame queued. **After presenting, never before drawing** — that
    /// ordering is §3.2's requirement and the whole reason [`Laid::queued`] is reported rather than
    /// waited on.
    pub fn flush_misses(&mut self, context: &ID3D11DeviceContext) -> Result<usize> {
        self.cache.flush_misses(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::offscreen::Offscreen;

    const CANDIDATES: &[&str] = &["Cascadia Mono", "Consolas", "Courier New", "Segoe UI"];
    const EM: u16 = 14;
    const TARGET: u32 = 256;
    const INK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
    const PAPER: [f32; 4] = [0.85, 0.85, 0.85, 1.0];

    fn painter_or_skip(what: &str) -> Option<(Offscreen, Painter)> {
        let off = match Offscreen::new(TARGET, TARGET) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("SKIPPED {what}: no D3D11 device ({e})");
                return None;
            }
        };
        match Painter::new(off.device(), CANDIDATES, EM) {
            Ok(mut p) => {
                assert!(p.prime(off.context()), "the placeholder must upload");
                Some((off, p))
            }
            Err(e) => {
                eprintln!("SKIPPED {what}: no usable font or pipeline ({e})");
                None
            }
        }
    }

    fn view_for(painter: &Painter, rows: u64, columns: u64) -> View {
        let mut v = View::new(painter.cell_width(), painter.row_height());
        v.grid_mut().set_total_rows(rows);
        v.hgrid_mut().set_columns(columns);
        v.set_viewport(TARGET as f32, TARGET as f32);
        v
    }

    /// The whole path, end to end: a viewport of rows becomes quads, the first frame draws
    /// placeholders without waiting, and the glyphs arrive afterwards.
    #[test]
    fn a_viewport_of_rows_reaches_real_pixels() {
        let Some((off, mut painter)) = painter_or_skip("a_viewport_of_rows_reaches_real_pixels")
        else {
            return;
        };
        let view = view_for(&painter, 4, 200);
        let lines = [
            "2026-08-07 09:14:02,131 INFO  Api.Controller  Query returned 412 rows",
            "2026-08-07 09:14:02,148 WARN  Api.Cache       miss for key 8821",
            "2026-08-07 09:14:03,004 ERROR Api.Controller  timeout after 30000ms",
            "2026-08-07 09:14:03,101 INFO  Api.Controller  retrying",
        ];

        // Frame one: nothing is resident. §3.2 says draw anyway.
        painter.begin_frame();
        let first = painter
            .lay_out(&view, INK, |row| {
                lines.get(row as usize).map(|s| s.to_string())
            })
            .expect("lay out");
        assert!(
            first.quads > 100,
            "only {} quads for four rows",
            first.quads
        );
        assert!(first.queued > 0, "a cold cache queued nothing");
        assert_eq!(first.rtl_runs, 0, "an ASCII log line has no RTL runs");

        off.clear(PAPER);
        painter.draw(off.context(), (TARGET, TARGET)).expect("draw");
        let placeholders = off.read_back().expect("read back");
        assert!(
            darkest(&placeholders) < 200,
            "the placeholder frame should not be blank paper"
        );

        // Then the rasterisation the frame deliberately did not wait for.
        assert!(painter.flush_misses(off.context()).expect("flush") > 0);

        painter.begin_frame();
        let second = painter
            .lay_out(&view, INK, |row| {
                lines.get(row as usize).map(|s| s.to_string())
            })
            .expect("lay out");
        assert_eq!(
            second.queued, 0,
            "a warm cache queued {} glyphs",
            second.queued
        );

        // **The warm frame draws *fewer* quads, and that is correct.** A glyph with no ink — every
        // space — is only known to be blank once it has been rasterised, so the cold frame drew a
        // placeholder box for each one and the warm frame draws nothing at all. See the module note
        // on what that looks like on screen.
        assert!(
            second.quads < first.quads,
            "the blank glyphs should stop being drawn once rasterised: {} then {}",
            first.quads,
            second.quads
        );

        off.clear(PAPER);
        painter.draw(off.context(), (TARGET, TARGET)).expect("draw");
        let real = off.read_back().expect("read back");
        assert!(
            darkest(&real) < 100,
            "black text on light paper should put something genuinely dark on the target"
        );
    }

    /// **§3.3's rule, asserted rather than assumed.** Every cluster starts on its column's x, so a
    /// line whose glyphs have any advance the cell does not have still lands on the grid. A pen
    /// advanced by shaped advances passes this on pure ASCII and fails the moment a glyph differs —
    /// so the assertion is on the *quad positions against the column grid*, not on a total width.
    #[test]
    fn every_cluster_lands_on_its_own_column() {
        let Some((off, mut painter)) = painter_or_skip("every_cluster_lands_on_its_own_column")
        else {
            return;
        };
        let view = view_for(&painter, 1, 200);
        let line = "GET /a/b?c=1 200 OK";

        painter.begin_frame();
        painter.lay_out_row(&view, line, 0.0, INK).expect("lay out");

        // Rebuild the expected column x for every cluster and require a quad to start there. The
        // cell's own `left` offsets ink within the cell, so the comparison allows it.
        let left = painter.cache.cell().left as f32;
        for cell in view.cells().cells(line) {
            if cell.width == 0 {
                continue;
            }
            let expected = view.hgrid().x_of_column(cell.cell) + left;
            assert!(
                painter
                    .instances()
                    .iter()
                    .any(|q| (q.pos[0] - expected).abs() < 0.01),
                "no quad starts at column {}'s x of {expected}",
                cell.cell
            );
        }
        drop(off);
    }

    /// **The gap, pinned.** An Arabic row shapes correctly and is placed in logical columns, which
    /// is wrong — and this must be visible rather than inferred. The day columns become bidi-aware,
    /// this test fails and its message says what to do.
    #[test]
    fn an_arabic_row_reports_that_its_runs_are_not_placed() {
        let Some((off, mut painter)) =
            painter_or_skip("an_arabic_row_reports_that_its_runs_are_not_placed")
        else {
            return;
        };
        let view = view_for(&painter, 1, 200);

        painter.begin_frame();
        let laid = painter
            .lay_out_row(&view, "ابب logged out", 0.0, INK)
            .expect("lay out");
        assert!(
            laid.rtl_runs > 0,
            "an Arabic run must be reported as unplaced while columns are logical — \
             if bidi column placement has landed, delete this test and the module note with it"
        );
        drop(off);
    }

    #[test]
    fn a_row_with_no_text_yet_draws_nothing_and_does_not_fail() {
        let Some((off, mut painter)) = painter_or_skip("a_row_with_no_text_yet_draws_nothing")
        else {
            return;
        };
        let view = view_for(&painter, 100, 200);

        painter.begin_frame();
        let laid = painter.lay_out(&view, INK, |_| None).expect("lay out");
        assert_eq!(laid, Laid::default());
        assert!(painter.instances().is_empty());
        // Drawing an empty frame is a no-op, not an error.
        painter.draw(off.context(), (TARGET, TARGET)).expect("draw");
    }

    /// A line scrolled off to the left contributes nothing, and one straddling the left edge is
    /// drawn from a negative x — `view.rs` decides that, and this asserts the painter honours it.
    #[test]
    fn a_horizontally_scrolled_row_draws_from_a_negative_x() {
        let Some((off, mut painter)) = painter_or_skip("a_horizontally_scrolled_row_draws") else {
            return;
        };
        let mut view = view_for(&painter, 1, 200);
        let line = "0123456789abcdefghijklmnopqrstuvwxyz";

        view.hgrid_mut().scroll_to_column(4);
        painter.begin_frame();
        painter.lay_out_row(&view, line, 0.0, INK).expect("lay out");
        let leftmost = painter
            .instances()
            .iter()
            .map(|q| q.pos[0])
            .fold(f32::INFINITY, f32::min);
        assert!(
            leftmost < painter.cell_width(),
            "the first drawn column is at {leftmost}, so nothing was scrolled"
        );
        drop(off);
    }

    fn darkest(pixels: &crate::gpu::offscreen::Pixels) -> u8 {
        let mut lowest = 255u8;
        for y in 0..TARGET {
            for x in 0..TARGET {
                let p = pixels.at(x, y);
                lowest = lowest.min(p[0]).min(p[1]).min(p[2]);
            }
        }
        lowest
    }
}
