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
//! is solved.
//!
//! **The design question that blocked it has been answered: a column is a *logical* position**
//! (`SPEC.md` §3.3, decided session 15). So `cell.rs` and `selection.rs` do not change, and the work
//! is confined to two places that convert: this module, and `View::position_at`. They must land
//! **together** — placing runs visually while the hit-test still answers logically puts the caret on
//! the wrong character, which is worse than today's honest gap.
//!
//! The first step is not in this module. Bidi cannot be resolved from a horizontal slice, and
//! `lay_out_row` shapes `&line[slice.bytes]` — so resolved levels must be computed for the whole
//! line and carried per row, alongside the column anchors `rows.rs` already builds.

use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

use crate::cell::ColumnAnchors;
use crate::glyphs::GlyphCache;
use crate::rows::RowSource;
use crate::shape::Shaper;
use crate::text::{Instance, TextPipeline};
use crate::view::View;
use crate::Result;
use crate::SELECTION_INK;

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
    /// Rows whose visible bytes hit [`RowSlice::truncated`](crate::view::RowSlice::truncated) and
    /// therefore drew short of their last visible column.
    pub truncated_rows: usize,
    /// **Rows that failed to shape and drew nothing.** See [`Painter::lay_out`] — this is the count
    /// that must never be silent, because the alternative it replaced was losing the whole frame.
    pub failed_rows: usize,
    /// Glyphs [`Painter::flush_misses`] rasterised after this frame was presented — the number that
    /// pairs with [`queued`](Self::queued). Set by the caller that owns the present, so it is zero
    /// on a `lay_out` that was not part of one.
    pub rasterised: usize,
}

impl Laid {
    fn merge(&mut self, other: Laid) {
        self.quads += other.quads;
        self.queued += other.queued;
        self.rtl_runs += other.rtl_runs;
        self.truncated_rows += other.truncated_rows;
        self.failed_rows += other.failed_rows;
        self.rasterised += other.rasterised;
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
    ///
    /// **A row that fails to shape costs that row and no others.** `shape` returns `Err` for a
    /// DirectWrite analysis or `GetGlyphs` failure, and propagating it from here used to abandon the
    /// whole frame: fifty rows already laid out were discarded and the caller, seeing `Err`, drew
    /// nothing. One malformed line would freeze a viewer that is following a live log — the failure
    /// mode a tail tool exists to not have. The same reasoning already governs the `None` arm one
    /// line up, and it does not become weaker because the cause is a font engine rather than a
    /// pending read. The row is skipped, [`Laid::failed_rows`] counts it, and the other forty-nine
    /// reach the screen.
    pub fn lay_out(&mut self, view: &View, tint: [f32; 4], source: &dyn RowSource) -> Result<Laid> {
        let mut total = Laid::default();
        let rows: Vec<(u64, f32)> = view.grid().visible().map(|p| (p.row, p.y)).collect();
        for (row, y) in rows {
            let Some(line) = source.row_text(row) else {
                continue;
            };
            let selected = source.row_selection(row);
            match self.lay_out_row(view, line, source.row_anchors(row), selected, y, tint) {
                Ok(laid) => total.merge(laid),
                Err(_) => total.failed_rows += 1,
            }
        }
        Ok(total)
    }

    /// One row. `y` is the row's top edge, viewport-relative, from [`crate::grid::PlacedRow`].
    pub fn lay_out_row(
        &mut self,
        view: &View,
        line: &str,
        anchors: &ColumnAnchors,
        selected: Option<core::ops::Range<usize>>,
        y: f32,
        tint: [f32; 4],
    ) -> Result<Laid> {
        let slice = view.slice_anchored(line, anchors);
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
            truncated_rows: usize::from(slice.truncated),
            ..Laid::default()
        };

        // **The column is carried, not re-derived per cluster**, and that is a measured
        // requirement rather than tidiness. `cell_at_byte` re-walks the line's graphemes from byte
        // zero on every call, so asking it once per cluster made a frame O(columns × scroll
        // offset): 50 rows of 32 KB lines scrolled to the middle measured **16.4 seconds** against
        // a 16.67 ms budget, and §10.3 puts exactly those lines in scope, citing klogg hanging
        // "deadly" on them as the behaviour to avoid.
        //
        // Carrying it is exact because `shape.rs` segments with the same `grapheme_indices(true)`
        // that `CellModel::cells` uses, and `byte_span` rounds the slice outwards to whole
        // clusters — so `shaped.clusters` *is* the slice's cluster partition, in logical order.
        let mut column = slice.column;
        let cell_width = view.hgrid().cell_width();

        for cluster in &shaped.clusters {
            let start = slice.bytes.start + cluster.span.byte;
            let cells = view
                .cells()
                .cluster_width(&line[start..start + cluster.span.byte_len]);
            let at = column;
            column += cells;

            // **A selected cluster is drawn in a different ink, and that is deliberately not a
            // highlight rectangle.** §3.2 plans one instanced draw carrying foreground *and*
            // background colour per instance, which is how a selection should eventually look, but
            // `Instance` has no background field and adding one means changing the glyph shader and
            // the offline `fxc` build. Re-tinting is the whole of the visual feedback until then:
            // it needs no new pipeline, and the per-row column range it consumes is the same thing
            // the eventual background pass will need. Recorded as a gap in `HANDOFF.md` rather than
            // presented as the finished look.
            let ink = match &selected {
                Some(range) if at >= range.start && at < range.end => SELECTION_INK,
                _ => tint,
            };

            // **After the advance, never before.** A cluster absorbed into a preceding ligature
            // draws nothing of its own but still occupies its cells, and skipping the advance
            // would shift the rest of the row left by one cluster per ligature.
            if cluster.glyph_count == 0 || cells == 0 {
                // `cells == 0` is a zero-width cluster — a bidi override, a joiner. It occupies no
                // column, so it draws nothing here; §13.4's reveal toggle is what gives it a cell,
                // and `byte_span` is what keeps it in the *copied* bytes per §5.6. Drawing it
                // anyway is not neutral: its glyph carries a full advance and lands on the next
                // character. See `last_x` below.
                continue;
            }

            let cluster_x = view.hgrid().x_of_column(at);
            // **The cluster's own cells bound its glyphs.** §3.3 gives the cell grid the last word
            // between clusters; nothing said what happens *within* one, and the answer is the
            // same authority. A cluster the cell model calls one cell wide can shape to several
            // full-advance glyphs — `a` followed by U+E0067 TAG LATIN SMALL LETTER G is one
            // width-1 cluster that shapes to two glyphs of ~8.2 px each — so an unbounded pen
            // walks the second glyph straight over the next column. Twenty tag characters paint a
            // box over each of the next twenty columns, which is §13.4's hidden-text vector
            // rendered as a viewer that can be made to lie about what a line says.
            let last_x = cluster_x + (cells - 1) as f32 * cell_width;

            let mut pen = cluster_x;
            for i in cluster.first_glyph..cluster.first_glyph + cluster.glyph_count {
                let offset = shaped.offsets[i];
                let before = self.cache.pending();
                if let Some(quad) = self.cache.quad(
                    shaped.glyphs[i],
                    (pen + offset.advance).min(last_x),
                    baseline_y - offset.ascender,
                    ink,
                ) {
                    self.instances.push(quad);
                    laid.quads += 1;
                }
                laid.queued += self.cache.pending() - before;
                // Within the cluster only, and never past its last cell. The next *cluster* starts
                // at its own column — see the module note on why the cell grid wins.
                pen = (pen + shaped.advances[i]).min(last_x);
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

    /// A [`RowSource`] over a plain list, with no anchors — which is the point: the default
    /// `row_anchors` returns the empty set, so these tests exercise the unanchored path and would
    /// fail if an empty anchor set ever stopped being a legitimate argument.
    struct Listed(Vec<String>);

    impl RowSource for Listed {
        fn row_text(&self, row: u64) -> Option<&str> {
            self.0.get(usize::try_from(row).ok()?).map(String::as_str)
        }
    }
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
            .lay_out(
                &view,
                INK,
                &Listed(lines.iter().map(|s| (*s).to_owned()).collect()),
            )
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
            .lay_out(
                &view,
                INK,
                &Listed(lines.iter().map(|s| (*s).to_owned()).collect()),
            )
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
        painter
            .lay_out_row(&view, line, ColumnAnchors::none_ref(), None, 0.0, INK)
            .expect("lay out");

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

    /// **A cluster's glyphs stay inside the cluster's cells, and §13.4 is why this matters.**
    ///
    /// `a` followed by U+E0067 TAG LATIN SMALL LETTER G is **one** cluster the cell model calls one
    /// cell wide, and it shapes to two glyphs each carrying a full advance — the second being
    /// `.notdef`, a hollow box with real ink. An unbounded within-cluster pen walks that box into
    /// the next column and paints it over the following character. Twenty tag characters put a box
    /// over each of the next twenty columns, so an attacker-supplied invisible blots out the text
    /// after it: §13.4's hidden-text vector rendered as a viewer that can be made to lie.
    #[test]
    fn a_clusters_glyphs_cannot_be_painted_over_the_next_column() {
        let Some((off, mut painter)) = painter_or_skip("a_clusters_glyphs_stay_in_their_cells")
        else {
            return;
        };
        let view = view_for(&painter, 1, 200);
        let line = format!("a{}SECRET", "\u{E0067}".repeat(20));
        assert_eq!(
            view.cells().cluster_width("a\u{E0067}"),
            1,
            "the fixture assumes the tag is absorbed into a one-cell cluster"
        );

        painter.begin_frame();
        painter
            .lay_out_row(&view, &line, ColumnAnchors::none_ref(), None, 0.0, INK)
            .expect("lay out");

        // Column 0 is the whole `a` + tags cluster. Nothing it emits may reach column 1, where the
        // `S` of SECRET is drawn.
        let column_1_x = view.hgrid().x_of_column(1) + painter.cache.cell().left as f32;
        let intruders = painter
            .instances()
            .iter()
            .filter(|q| q.pos[0] >= column_1_x)
            .count();
        let letters = "SECRET".len();
        assert!(
            intruders <= letters,
            "{intruders} quads land at or past column 1, which holds only {letters} characters — \
             the cluster's glyphs are being painted over the text after it"
        );
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
            .lay_out_row(
                &view,
                "ابب logged out",
                ColumnAnchors::none_ref(),
                None,
                0.0,
                INK,
            )
            .expect("lay out");
        assert!(
            laid.rtl_runs > 0,
            "an Arabic run must be reported as unplaced while columns are logical — \
             if bidi column placement has landed, delete this test and the module note with it"
        );
        drop(off);
    }

    /// **The test the existing ones could not be: a wide line, scrolled a long way in.**
    ///
    /// Every other test here uses lines of 30–70 characters at or near column zero, where an
    /// O(columns × scroll offset) layout is indistinguishable from a linear one. Adversarial review
    /// measured the real shape: a screenful of 32 KB lines scrolled to the middle took **16.4
    /// seconds** for one frame, because the column of every cluster was re-derived by walking the
    /// line's graphemes from byte zero. §10.3 puts exactly these lines in scope and cites klogg
    /// hanging "deadly" on them as the behaviour to avoid.
    ///
    /// **⚠ This asserts a ratio, not a duration, and the first version of it asserted a duration.**
    /// That version measured 0.21 s alone and 0.60 s inside the full suite, and failed — this
    /// machine carries a variable ~40% background load, which `experiments/g3-d3d11` spent two
    /// sessions establishing and which produced 96, 112, 126, 139, 154 and 297 ms for one binary.
    /// The project's own rule came out of that: decide with **paired interleaved A/B**, never with
    /// an absolute.
    ///
    /// So both arms run in one process, alternating, at the *same* scroll offset with only the
    /// number of visible columns differing. The defect's cost is `columns × offset`, so a ten-fold
    /// difference in columns showed up as a ten-fold difference in time; the fix's cost is
    /// dominated by the offset, which both arms share, so they land on top of each other. Load
    /// affects both arms equally and cancels.
    ///
    /// **What this does not assert is that the frame is fast enough.** It is not: the residual
    /// linear-in-offset walk is ~200 ms for this frame against a 16.67 ms budget. That is recorded
    /// as an open item rather than hidden behind a passing test — see the module note.
    #[test]
    fn the_layout_cost_does_not_scale_with_the_number_of_visible_columns() {
        let Some((off, mut painter)) = painter_or_skip("the_layout_cost_does_not_scale") else {
            return;
        };
        // §10.3's supported inline size, as an ordinary wide record — **opened with one non-ASCII
        // character**, which is load-bearing. `CellModel::is_column_per_byte` makes an all-ASCII
        // line's column lookups O(1), so an ASCII fixture drove both arms of this ratio into the
        // tens of microseconds, where the ratio is measuring scheduler noise and fails at random
        // under full-suite load. The property under test — that the column is carried across
        // clusters rather than re-derived per cluster — only has a cost to measure on a line that
        // actually walks.
        let line = format!("— {}", "abcdefgh 0123456789 ".repeat(32 * 1024 / 20));
        assert!(!crate::cell::CellModel::is_column_per_byte(&line));
        let cell_w = painter.cell_width();

        let frame = |painter: &mut Painter, columns: usize| {
            let mut view = view_for(painter, 8, line.len() as u64);
            view.set_viewport(columns as f32 * cell_w, 8.0 * painter.row_height());
            view.hgrid_mut().scroll_to_column(16_384);
            let started = std::time::Instant::now();
            painter.begin_frame();
            for row in 0..8 {
                painter
                    .lay_out_row(
                        &view,
                        &line,
                        ColumnAnchors::none_ref(),
                        None,
                        row as f32 * painter.row_height(),
                        INK,
                    )
                    .expect("lay out");
            }
            assert!(!painter.instances().is_empty(), "nothing was drawn");
            started.elapsed()
        };

        // Interleaved, never one arm and then the other: the fixed-order comparison is the mistake
        // that made session 5 report concurrency as 11% slower when it was faster.
        let (mut wide, mut narrow) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            wide.push(frame(&mut painter, 240));
            narrow.push(frame(&mut painter, 24));
        }
        wide.sort();
        narrow.sort();
        let ratio = wide[2].as_secs_f64() / narrow[2].as_secs_f64().max(1e-9);

        assert!(
            ratio < 3.0,
            "ten times the columns cost {ratio:.1}x the time ({:?} against {:?}) — \
             the layout is scaling with the column count again, which is the per-cluster walk",
            wide[2],
            narrow[2]
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
        let laid = painter
            .lay_out(&view, INK, &Listed(Vec::new()))
            .expect("lay out");
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
        painter
            .lay_out_row(&view, line, ColumnAnchors::none_ref(), None, 0.0, INK)
            .expect("lay out");
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

    /// **Why [`Laid::failed_rows`] has no test that exercises it, recorded as evidence rather than
    /// asserted as an excuse.**
    ///
    /// `lay_out` containing a row's shaping failure instead of propagating it is a real change —
    /// propagating cost the whole frame — but a negative control on it does not fire, and by this
    /// project's rule that is a statement about the tests. So the question was put directly: what
    /// legal `&str` makes DirectWrite fail? These six are the hostile shapes that plausibly could —
    /// 5,000 combining marks in one run (the `GetGlyphs` buffer-doubling path, `MAX_RETRIES`),
    /// private-use, unassigned and noncharacter code points, tag characters, and 2,000 stacked bidi
    /// overrides. **All six shape.** The error path is reachable only on a system-level failure — a
    /// COM error, a corrupt font, allocation failure — which no test can produce without a seam.
    ///
    /// That is worth knowing beyond this test: it means the containment protects against something
    /// rare and catastrophic rather than something routine, and it is why no seam was added to prove
    /// it. If a fixture here ever *does* error, `failed_rows` becomes testable and should get a real
    /// test that day.
    #[test]
    fn hostile_text_shapes_rather_than_failing() {
        let Some((_off, mut painter)) = painter_or_skip("hostile_text_shapes") else {
            return;
        };
        let cases: Vec<(&str, String)> = vec![
            (
                "many marks one run",
                format!("a{}", "\u{0301}".repeat(5000)),
            ),
            ("private use", "\u{E000}".repeat(100)),
            ("unassigned", "\u{0378}".repeat(100)),
            ("noncharacter", "\u{FFFE}\u{FFFF}".repeat(50)),
            ("tags", format!("a{}", "\u{E0067}".repeat(500))),
            ("deep overrides", "\u{202D}".repeat(2000)),
        ];
        let view = view_for(&painter, 4, 100_000);
        for (name, line) in &cases {
            painter.begin_frame();
            painter
                .lay_out_row(&view, line, ColumnAnchors::none_ref(), None, 0.0, INK)
                .unwrap_or_else(|e| panic!("{name} failed to shape: {e:?} — see this test's note"));
        }

        // And a frame of them reports no failures, which is the contract `lay_out` now owes.
        painter.begin_frame();
        let laid = painter
            .lay_out(
                &view,
                INK,
                &Listed(cases.iter().map(|(_, l)| l.clone()).collect()),
            )
            .expect("a frame of hostile rows is still a frame");
        assert_eq!(laid.failed_rows, 0);
        assert!(laid.quads > 0);
    }
}
