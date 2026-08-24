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
use crate::highlight::Span;
use crate::rows::RowSource;
use crate::shape::Shaper;
use crate::text::{Instance, TextPipeline, MODE_SOLID};
use crate::theme::theme;
use crate::view::View;
use crate::Result;

/// What colours one row, gathered because the three arrive together and mean nothing apart.
///
/// The painter needs the plain ink, the columns the selection covers and the byte spans a rule or a
/// search claimed — and it resolves them against each other per cluster, which is the argument for
/// them being one thing rather than three parameters that happen to be adjacent.
///
/// Not `Copy`, because `Range` is not — deliberately, upstream.
#[derive(Clone, Debug)]
pub struct Colours<'a> {
    /// The colour a cluster no span claimed is drawn in.
    pub tint: [f32; 4],
    /// The **cell columns** selected on this row. See
    /// [`RowSource::row_selection`](crate::rows::RowSource::row_selection).
    pub selected: Option<core::ops::Range<usize>>,
    /// Coloured runs in **byte** offsets, sorted and non-overlapping — what
    /// [`Highlighter::line`](crate::highlight::Highlighter::line) and a search's matches produce.
    pub spans: &'a [Span],
}

impl Colours<'_> {
    /// Plain text in `tint`: no selection, no spans.
    pub fn plain(tint: [f32; 4]) -> Self {
        Self {
            tint,
            selected: None,
            spans: &[],
        }
    }
}

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
    pub fn merge(&mut self, other: Laid) {
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
    /// The **chrome** face and its own atlas — `UI-DESIGN.md` §1.1's menus, toolbar and status bar
    /// are drawn in the system UI font, not in the grid's monospace.
    ///
    /// A second cache rather than a second face in the first one, because [`crate::atlas`] gives
    /// every glyph an identical slot sized by measuring one face. That uniformity is what makes
    /// eviction O(1) with no repacking, and it is exactly what a proportional face cannot share: a
    /// cell wide enough for `W` wastes most of the sheet on `i`, and one sized for the monospace
    /// grid would clip half of Segoe UI.
    chrome: GlyphCache,
    pipeline: TextPipeline,
    px_per_em: u16,
    instances: Vec<Instance>,
    /// The chrome's quads, drawn after [`Painter::instances`] with the chrome sheet bound — a
    /// second draw call, and the reason the two cannot share a buffer.
    chrome_instances: Vec<Instance>,
    /// One row's coloured runs, reused for every row of the frame. See
    /// [`RowSource::row_spans`](crate::rows::RowSource::row_spans) for why it is filled rather than
    /// returned.
    spans: Vec<Span>,
}

impl Painter {
    /// `candidates` and `px_per_em` are the **grid's** face and scale; `chrome` and `chrome_px` are
    /// the system UI font's, from `SPI_GETNONCLIENTMETRICS`. The two are independent: the grid
    /// follows the monitor's DPI, the chrome follows what the user set Windows' menus to.
    pub fn new(
        device: &ID3D11Device,
        candidates: &[&str],
        px_per_em: u16,
        chrome: &[&str],
        chrome_px: u16,
    ) -> Result<Self> {
        Ok(Self {
            shaper: Shaper::new()?,
            cache: GlyphCache::new(device, candidates, px_per_em)?,
            chrome: GlyphCache::new(device, chrome, chrome_px)?,
            pipeline: TextPipeline::new(device)?,
            px_per_em,
            instances: Vec::new(),
            chrome_instances: Vec::new(),
            spans: Vec::new(),
        })
    }

    /// The chrome face's line height — what a menu row, a toolbar band or the status bar is tall.
    /// The chrome face, for a caller that needs to ask what it contains.
    pub fn chrome_face(&self) -> &crate::raster::Face {
        self.chrome.face()
    }

    /// The **grid** face — the monospace one the rows and the column header are drawn in.
    ///
    /// Exposed for the same reason [`chrome_face`](Self::chrome_face) is: so a test can ask whether
    /// a glyph this face is about to draw actually exists in it. The header carries §E22's sort
    /// triangles and is laid out in *this* face, not the chrome's, so proving them there proves
    /// nothing here.
    pub fn grid_face(&self) -> &crate::raster::Face {
        self.cache.face()
    }

    pub fn chrome_line_height(&self) -> f32 {
        let ink = self.chrome.cell().height as f32;
        self.chrome
            .face()
            .line_height(self.chrome.px_per_em())
            .ceil()
            .max(ink)
    }

    /// How wide `text` is in the chrome face, without drawing it — for a hit rectangle, or for
    /// right-aligning an accelerator against a menu's edge.
    ///
    /// Shapes, which is the only honest way to answer: a proportional face's width is not a
    /// character count times anything.
    pub fn chrome_measure(&self, text: &str) -> f32 {
        self.shaper
            .shape(self.chrome.face(), text, self.chrome.px_per_em())
            .map(|shaped| shaped.advances.iter().sum())
            .unwrap_or(0.0)
    }

    /// Draws `text` in the chrome face with its top-left at `(x, y)`, and reports how wide it was.
    ///
    /// **A pen walked by the shaper's own advances** — the opposite of [`Painter::lay_out_row`],
    /// which places every cluster at its column because §3.3 says the cell grid wins. That rule
    /// exists so one fallback glyph cannot knock a log line out of column for the rest of its
    /// length; it has nothing to say about a menu label, where honouring the advances is the whole
    /// point of using a proportional face.
    pub fn chrome_run(&mut self, text: &str, x: f32, y: f32, tint: [f32; 4]) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        let px = self.chrome.px_per_em();
        let Ok(shaped) = self.shaper.shape(self.chrome.face(), text, px) else {
            return 0.0;
        };
        // As in `lay_out_row`: the cell's `top` is the ink's offset from the baseline, negative
        // above it, and `quad` adds it back — so subtracting lands the ink on the row's top edge.
        // Half the leading above and half below, for the reason given there.
        let leading = (self.chrome_line_height() - self.chrome.cell().height as f32) * 0.5;
        let baseline_y = y + leading - self.chrome.cell().top as f32;
        let mut pen = x;
        for (i, glyph) in shaped.glyphs.iter().enumerate() {
            let offset = shaped.offsets[i];
            if let Some(quad) = self.chrome.quad(
                *glyph,
                pen + offset.advance,
                baseline_y - offset.ascender,
                tint,
            ) {
                self.chrome_instances.push(quad);
            }
            pen += shaped.advances[i];
        }
        pen - x
    }

    /// Drops chrome text already queued inside the rectangle, so a surface drawn *later* actually
    /// covers it.
    ///
    /// **Chrome text is a second instance buffer, drawn after every fill** — that is what lets a
    /// field's ink sit over the field's own background without ordering the two by hand. The cost
    /// is that a box drawn later cannot hide the text beneath it: the command bar's placeholder
    /// reads straight through an open menu, and the gutter's line numbers through the palette.
    /// A later surface calls this over its own box before drawing its own text, and the two rules
    /// stop fighting.
    ///
    /// Whole quads only — a glyph is either under the box or it is not. A glyph straddling the
    /// edge is kept, which is right for a box drawn against a boundary and never noticeable for
    /// one drawn over the middle of something.
    pub fn occlude_chrome(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.chrome_instances.retain(|q| {
            let (qx, qy) = (q.pos[0], q.pos[1]);
            let (qw, qh) = (q.size[0], q.size[1]);
            qx < x || qy < y || qx + qw > x + w || qy + qh > y + h
        });
    }

    /// Uploads the placeholder. Until this succeeds a missing glyph draws nothing rather than a box.
    pub fn prime(&mut self, context: &ID3D11DeviceContext) -> bool {
        let grid = self.cache.prime(context);
        let chrome = self.chrome.prime(context);
        grid && chrome
    }

    /// The measured cell — **what [`View`]'s metrics must be set from.** §3.1 requires integer cell
    /// advances derived from the face at the current scale, so these are the only honest source for
    /// `View::set_metrics`, and a constant would drift from the font on any DPI change.
    pub fn cell_width(&self) -> f32 {
        self.cache.cell().width as f32
    }

    /// The distance from one row's baseline to the next — the **face's designed line height**, not
    /// the height of the ink.
    ///
    /// [`Rasteriser::measure_cell`](crate::raster::Rasteriser::measure_cell) measures the tight
    /// bounding box of the printable ASCII, top of the tallest ascender to bottom of the deepest
    /// descender, with no space between lines in it at all. That is the right box to pack an atlas
    /// with and the wrong number to advance a row by: used as the row height it put a `g` hard
    /// against the `l` below it, and a screenful read as one solid block.
    ///
    /// The ink box is still the floor. A face that reports a line height smaller than its own ink —
    /// or reports nothing usable — gets the ink box rather than glyphs that overlap outright.
    pub fn row_height(&self) -> f32 {
        let ink = self.cache.cell().height as f32;
        self.cache
            .face()
            .line_height(self.cache.px_per_em())
            .ceil()
            .max(ink)
    }

    /// The space above the ink inside a row — half of what the line height adds to the ink box.
    ///
    /// Rounded down so the ink stays on a whole pixel: a half-pixel baseline is a blurred one, and
    /// the atlas rasterised these glyphs for an integer grid.
    fn half_leading(&self) -> f32 {
        ((self.row_height() - self.cache.cell().height as f32) / 2.0).floor()
    }

    /// Starts a frame: drops the previous frame's quads and its miss list.
    pub fn begin_frame(&mut self) {
        self.instances.clear();
        self.chrome_instances.clear();
        self.cache.begin_frame();
        self.chrome.begin_frame();
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
        let inset = view.top_inset();
        let header_px = view.header_px();
        let rows: Vec<(u64, f32)> = view
            .grid()
            .visible()
            .map(|p| (p.row, p.y + inset))
            .collect();
        // Taken out and put back so the row loop can hold it mutably while `self` is borrowed for
        // the layout. One `Vec` for the frame is the point; taking it does not allocate.
        let mut spans = std::mem::take(&mut self.spans);
        let t = theme();
        let gutter = view.gutter_px();
        let cell_w = self.cell_width();
        for (row, y) in rows {
            // The gutter: a mark and the physical line number, right-aligned, in a quieter ink.
            if gutter > 0.0 {
                if let Some(colour) = source.row_mark(row) {
                    self.instances.push(Instance {
                        pos: [0.0, y],
                        size: [cell_w * 0.5, view.grid().row_height()],
                        tint: colour,
                        mode: MODE_SOLID,
                        ..Instance::default()
                    });
                    total.quads += 1;
                }
                if let Some((glyph, ink)) = source.row_glyph(row) {
                    let mut text = [0u8; 4];
                    self.spans = spans;
                    let _ = self.lay_out_at(
                        view,
                        cell_w,
                        y,
                        glyph.encode_utf8(&mut text),
                        Colours::plain(ink),
                    );
                    spans = std::mem::take(&mut self.spans);
                }
                if let Some(n) = source.row_number(row) {
                    let text = n.to_string();
                    let width = ((gutter / cell_w) as usize).saturating_sub(1);
                    let x = (width.saturating_sub(text.len())) as f32 * cell_w;
                    self.spans = spans;
                    let _ = self.lay_out_at(view, x, y, &text, Colours::plain(t.gutter_ink));
                    spans = std::mem::take(&mut self.spans);
                }
            }
            let Some(line) = source.row_text(row) else {
                continue;
            };
            source.row_spans(row, &mut spans);
            let colours = Colours {
                tint,
                selected: source.row_selection(row),
                spans: &spans,
            };
            let from = self.instances.len();
            match self.lay_out_row(view, line, source.row_anchors(row), colours, y) {
                Ok(laid) => total.merge(laid),
                Err(_) => total.failed_rows += 1,
            }
            // Rows start right of the gutter. Moved after the layout rather than threaded through
            // it — every x in `lay_out_row` is a column's, and the gutter is not a column.
            if gutter > 0.0 {
                for instance in &mut self.instances[from..] {
                    instance.pos[0] += gutter;
                }
            }
        }
        // The header band, when there is one: a filled strip and the column names in it. Laid out
        // like a row, in the same cells, so it lines up with what is under it. **After the rows**: a
        // row scrolled partly under the bands is covered by their fills rather than drawn over
        // them — the rows are not clipped to the grid's band, and this order is what does it.
        if let Some(header) = source.header().filter(|_| header_px > 0.0) {
            let chrome = view.chrome_px();
            self.instances.push(Instance {
                pos: [0.0, chrome],
                size: [view.gutter_px() + view.hgrid().viewport_px(), header_px],
                tint: t.header_bg,
                mode: MODE_SOLID,
                ..Instance::default()
            });
            total.quads += 1;
            let columns = source.header_columns();
            if columns.is_empty() {
                // No real columns: the old single-string path, laid out in the grid's cells.
                spans.clear();
                let from = self.instances.len();
                match self.lay_out_row(
                    view,
                    header,
                    ColumnAnchors::none_ref(),
                    Colours::plain(t.header_ink),
                    chrome,
                ) {
                    Ok(laid) => total.merge(laid),
                    Err(_) => total.failed_rows += 1,
                }
                for instance in &mut self.instances[from..] {
                    instance.pos[0] += view.gutter_px();
                }
            } else {
                total.quads += self.lay_out_header_boxes(view, &columns, chrome, header_px);
            }
            // The rule under the band — `UI-DESIGN.md` §2.5. **This, not the fill, is what makes
            // the strip read as a header.** The fill is deliberately quiet, because principle 5
            // wants the user's own highlight colours to be the loudest thing on screen; and under
            // High Contrast the fill is the row background exactly, so the rule is all there is.
            // One instance, whatever the column count.
            self.instances.push(Instance {
                pos: [0.0, chrome + header_px - 1.0],
                size: [view.gutter_px() + view.hgrid().viewport_px(), 1.0],
                tint: t.header_rule,
                mode: MODE_SOLID,
                ..Instance::default()
            });
            total.quads += 1;
        }
        // The command bar, if the source draws one — V14. Last, so its fills sit over whatever a
        // partial top row put under them, and its text over its fills.
        if view.chrome_px() > 0.0 {
            self.spans = spans;
            source.draw_chrome(self, view);
            spans = std::mem::take(&mut self.spans);
        }
        self.spans = spans;
        Ok(total)
    }

    /// One row. `y` is the row's top edge, viewport-relative, from [`crate::grid::PlacedRow`].
    ///
    /// [`Colours::spans`] are consumed twice: once up front for the backgrounds, and once along the
    /// cluster walk for the ink.
    pub fn lay_out_row(
        &mut self,
        view: &View,
        line: &str,
        anchors: &ColumnAnchors,
        colours: Colours<'_>,
        y: f32,
    ) -> Result<Laid> {
        let Colours {
            tint,
            selected,
            spans,
        } = colours;
        let t = theme();
        let slice = view.slice_anchored(line, anchors);
        if slice.bytes.is_empty() {
            return Ok(Laid::default());
        }
        let text = &line[slice.bytes.clone()];
        let shaped = self.shaper.shape(self.cache.face(), text, self.px_per_em)?;

        // The cell box's `top` is the ink's offset from the baseline and is negative above it, and
        // `quad` adds it back. Subtracting it here therefore lands the cell exactly on the row's top
        // edge — the one place screen space and font metrics meet, resolved once.
        //
        // **Half of the leading goes above the ink and half below.** The row is now the face's line
        // height rather than the height of the ink (see [`Painter::row_height`]), so there is space
        // to place; putting all of it under the glyphs would sit every line hard against the row
        // above and leave a gap beneath, which reads as badly as no leading did.
        let baseline_y = y + self.half_leading() - self.cache.cell().top as f32;

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

        // **Backgrounds first, because ordering in the instance buffer is what puts them
        // underneath** — `text.rs` says so at [`MODE_SOLID`], and emitting one after the glyphs it
        // sits behind paints over them instead.
        //
        // **One quad per span, not one per cluster.** A span's column extent comes from two anchored
        // `cell_at_byte` lookups, which is what `ColumnAnchors` was built for — walking the clusters
        // to accumulate it would emit a quad per character and pay the walk twice.
        //
        // Clipped to the slice, because a span may name bytes off either edge of a horizontally
        // scrolled viewport — and a whole-line rule (§7.1) always does.
        let row_height = view.grid().row_height();
        for span in spans {
            let Some(bg) = span.bg else {
                continue;
            };
            let from_byte = span.start.max(slice.bytes.start);
            let to_byte = span.end.min(slice.bytes.end);
            if from_byte >= to_byte {
                continue;
            }
            let from = view.cells().cell_at_byte_anchored(line, from_byte, anchors);
            let to = view.cells().cell_at_byte_anchored(line, to_byte, anchors);
            if to <= from {
                continue;
            }
            self.instances.push(Instance {
                pos: [view.hgrid().x_of_column(from), y],
                size: [(to - from) as f32 * cell_width, row_height],
                tint: bg,
                mode: MODE_SOLID,
                ..Instance::default()
            });
            laid.quads += 1;
        }

        // The selection's fill, over the spans' backgrounds and under the ink: selection is the
        // user's live act and highlighting is a standing instruction, so where both claim a cell
        // the selection is the one that must be seen. The range arrives in cell columns, so the
        // only clamping needed is to the columns the slice actually shows.
        if let Some(range) = &selected {
            let end_column = view
                .cells()
                .cell_at_byte_anchored(line, slice.bytes.end, anchors);
            let from = range.start.max(slice.column);
            let to = range.end.min(end_column);
            if to > from {
                self.instances.push(Instance {
                    pos: [view.hgrid().x_of_column(from), y],
                    size: [(to - from) as f32 * cell_width, row_height],
                    tint: t.selection_bg,
                    mode: MODE_SOLID,
                    ..Instance::default()
                });
                laid.quads += 1;
            }
        }

        // The spans again, this time for the ink. Clusters arrive in logical order and spans are
        // sorted and non-overlapping, so one forward cursor answers every cluster — no search, and
        // no dependence on how many spans the row has.
        let mut span_at = 0usize;

        for cluster in &shaped.clusters {
            let start = slice.bytes.start + cluster.span.byte;
            let cells = view
                .cells()
                .cluster_width(&line[start..start + cluster.span.byte_len]);
            let at = column;
            column += cells;

            // **A selected cluster keeps the ink it already had** — the fill emitted above is what
            // says "selected", exactly as it does in every standard application; the first shipped
            // form re-tinted the letters instead and the owner read it as the font changing
            // colour. The one exception is High Contrast, where [`Theme::selection_ink`] carries
            // the system background so text stays legible on the system-highlight fill.
            while span_at < spans.len() && spans[span_at].end <= start {
                span_at += 1;
            }
            let claimed = spans
                .get(span_at)
                .filter(|span| span.start <= start)
                .and_then(|span| span.fg);
            let ink = match &selected {
                Some(range) if at >= range.start && at < range.end => {
                    t.selection_ink.unwrap_or(claimed.unwrap_or(tint))
                }
                _ => claimed.unwrap_or(tint),
            };

            // **After the advance, never before.** A cluster absorbed into a preceding ligature
            // draws nothing of its own but still occupies its cells, and skipping the advance
            // would shift the rest of the row left by one cluster per ligature.
            // §13.4's reveal toggle: an invisible given a cell draws a marker there, never its own
            // glyphs — those carry a full advance and would land on the next character. See
            // `CellModel::is_revealed`.
            if view
                .cells()
                .is_revealed(&line[start..start + cluster.span.byte_len])
            {
                let inset = cell_width * 0.25;
                self.instances.push(Instance {
                    pos: [view.hgrid().x_of_column(at) + inset, y + row_height * 0.3],
                    size: [cell_width - 2.0 * inset, row_height * 0.4],
                    tint: t.reveal_mark,
                    mode: MODE_SOLID,
                    ..Instance::default()
                });
                laid.quads += 1;
                continue;
            }
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

    /// A filled rectangle in `tint`, in viewport pixels — the chrome's backgrounds, a caret, a
    /// mark under a composition.
    /// The column header as boxes — `UI-DESIGN.md` §2.5. Reports the quads added.
    ///
    /// **Each label is drawn in the chrome face at its own column's edge**, which is what makes the
    /// strip read as a list header rather than as the first line of the log. The face is
    /// proportional, so nothing here counts cells to align text — the *box* is placed in cells and
    /// the label simply starts inside it. That costs the character-grid alignment the padded-string
    /// header had, and it is not a loss worth mourning: a label sitting over its own column is what
    /// alignment was for.
    ///
    /// **The dividers are the point as much as the labels are.** They separate the boxes, and they
    /// stand exactly where `Document::header_cell` resolves a resize drag, so the line you can see
    /// is the line you can grab. Drawing them made an affordance that already worked discoverable
    /// for the first time.
    fn lay_out_header_boxes(
        &mut self,
        view: &crate::view::View,
        columns: &[crate::rows::HeaderColumn],
        top: f32,
        height: f32,
    ) -> usize {
        let t = crate::theme::theme();
        let gutter = view.gutter_px();
        let width = view.hgrid().viewport_px();
        let cell_w = view.hgrid().cell_width();
        let chrome_h = self.chrome_line_height();
        // The label sits on the baseline the band centres, and a hair in from its own edge so it is
        // not welded to the divider on its left.
        let text_y = top + ((height - chrome_h) * 0.5).floor().max(0.0);
        let pad = (cell_w * 0.5).max(3.0);
        let mut quads = 0;

        for (i, column) in columns.iter().enumerate() {
            let x = gutter + view.hgrid().x_of_column(column.start);
            let right = gutter + view.hgrid().x_of_column(column.start + column.cells);

            // Wholly scrolled off: nothing to draw, and no divider either.
            if right <= gutter || x >= gutter + width {
                continue;
            }

            // The sort indicator, right-aligned in the box, drawn first so the title can be cut
            // short of it rather than run underneath it. §11.4 is why it earns the space: sorting
            // is a mode, and this is the only thing that says the rows are not in file order.
            let mut room = (right - x - pad * 2.0).max(0.0);
            if let Some(descending) = column.sort {
                let mark = if descending { "\u{25BC}" } else { "\u{25B2}" };
                let mark_w = self.chrome_measure(mark);
                if room > mark_w {
                    self.chrome_run(mark, right - pad - mark_w, text_y, t.header_ink);
                    quads += 1;
                    room -= mark_w + pad;
                }
            }

            // The title, cut to what its own box will hold. A header that overruns into the next
            // column is worse than one that is short.
            let title = self.fit_to_width(&column.title, room);
            if !title.is_empty() {
                self.chrome_run(&title, x + pad, text_y, t.header_ink);
                quads += 1;
            }

            // The divider, for every column but the last — the band's own rule closes the strip.
            //
            // **At the end of the column's data, not at the edge of its box.** The gap between
            // columns belongs to neither, and `Document::column_boundaries` records the resize
            // boundary at the data's edge — it adds the width, notes the edge, and only then adds
            // the gap. Drawing at the box edge put the line `GAP` cells to the right of the drag
            // target it advertises, which a test caught before the window did.
            let edge = gutter + view.hgrid().x_of_column(column.start + column.content);
            if i + 1 < columns.len() && edge > gutter && edge < gutter + width {
                self.fill(edge, top + 2.0, 1.0, (height - 4.0).max(1.0), t.header_rule);
                quads += 1;
            }
        }
        quads
    }

    /// The longest prefix of `text` that fits `room` pixels in the chrome face.
    ///
    /// Measured rather than counted: the face is proportional, so there is no cell count that
    /// answers this. Cut at a character boundary, and give back nothing rather than a single
    /// letter when the box is too narrow to say anything useful.
    fn fit_to_width(&mut self, text: &str, room: f32) -> String {
        if room <= 0.0 {
            return String::new();
        }
        if self.chrome_measure(text) <= room {
            return text.to_owned();
        }
        let mut cut = text.len();
        while cut > 0 {
            match text.get(..cut) {
                Some(slice) if self.chrome_measure(slice) <= room => {
                    return if slice.chars().count() > 1 {
                        slice.to_owned()
                    } else {
                        String::new()
                    };
                }
                _ => cut -= 1,
            }
        }
        String::new()
    }

    pub fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        self.instances.push(Instance {
            pos: [x, y],
            size: [w, h],
            tint,
            mode: MODE_SOLID,
            ..Instance::default()
        });
    }

    /// One line of text at an arbitrary origin, in the row grid's cells — chrome text: a field's
    /// contents, a chip, a label. Laid out exactly as a row is (same shaper, same cell model, same
    /// spans and selection) against a private unscrolled view, then moved to `x`. Returns what a
    /// row would; the caller's `Colours` decide the ink, and a span with a background is how a
    /// field draws its selection.
    pub fn lay_out_at(
        &mut self,
        view: &View,
        x: f32,
        y: f32,
        text: &str,
        colours: Colours<'_>,
    ) -> Result<Laid> {
        let mut own = View::new(self.cell_width(), self.row_height());
        *own.cells_mut() = *view.cells();
        let cells = view.cells().cell_count(text) as u64 + 1;
        own.hgrid_mut().set_columns(cells);
        own.set_viewport(
            cells as f32 * self.cell_width() + 1.0,
            self.row_height() * 2.0,
        );
        own.grid_mut().set_total_rows(1);
        let from = self.instances.len();
        let laid = self.lay_out_row(&own, text, ColumnAnchors::none_ref(), colours, y)?;
        for instance in &mut self.instances[from..] {
            instance.pos[0] += x;
        }
        Ok(laid)
    }

    /// The frame's quads, for a caller that wants to inspect or extend them before drawing.
    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    /// Draws the frame. The render target and viewport must already be set.
    /// How many instances the frame holds so far — the start of a pane, for [`Painter::shift`].
    pub fn mark(&self) -> usize {
        self.instances.len()
    }

    /// Moves every instance laid out since `from` by `(dx, dy)`: a pane drawn at an offset.
    pub fn shift(&mut self, from: usize, dx: f32, dy: f32) {
        let from = from.min(self.instances.len());
        for instance in &mut self.instances[from..] {
            instance.pos[0] += dx;
            instance.pos[1] += dy;
        }
    }

    pub fn draw(&self, context: &ID3D11DeviceContext, viewport: (u32, u32)) -> Result<()> {
        self.pipeline
            .draw(context, self.cache.sheet(), viewport, &self.instances)?;
        // Chrome second, so it is over the grid rather than under it.
        self.pipeline.draw(
            context,
            self.chrome.sheet(),
            viewport,
            &self.chrome_instances,
        )
    }

    /// Rasterises what this frame queued. **After presenting, never before drawing** — that
    /// ordering is §3.2's requirement and the whole reason [`Laid::queued`] is reported rather than
    /// waited on.
    pub fn flush_misses(&mut self, context: &ID3D11DeviceContext) -> Result<usize> {
        Ok(self.cache.flush_misses(context)? + self.chrome.flush_misses(context)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::offscreen::Offscreen;
    use crate::theme::Theme;

    const CANDIDATES: &[&str] = &["Cascadia Mono", "Consolas", "Courier New", "Segoe UI"];
    const EM: u16 = 14;
    const CHROME: &[&str] = &["Segoe UI Variable Text", "Segoe UI", "Tahoma", "Arial"];
    const CHROME_EM: u16 = 12;
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
        match Painter::new(off.device(), CANDIDATES, EM, CHROME, CHROME_EM) {
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

    /// The chrome face must actually contain the markers the chrome draws.
    ///
    /// `GetGlyphIndices` maps a codepoint the face lacks to **0**, `.notdef`, and this cache has one
    /// face with no per-glyph fallback — so a marker the face does not have is a box on screen, not
    /// a substitution. That is what `▸`, `▼` and `▾` became when the chrome moved from Cascadia
    /// Mono to Segoe UI Variable Text, and nothing but looking at the window said so.
    /// The **grid** face must contain the markers the *header* draws — which is a different face
    /// and therefore a different question from the test below it.
    ///
    /// `Layout::header` inserts `▲` or `▼` beside the sorted column's title, and the header is laid
    /// out by `lay_out_row` in the grid's monospace face. Everything proven about the chrome face
    /// is irrelevant to it. Nothing tested this, so a sorted column has been one absent glyph away
    /// from drawing a box since E22 landed, with no way to find out but sorting a column and
    /// looking.
    #[test]
    fn the_grid_face_has_the_markers_the_header_draws() {
        let Some((_off, p)) = painter_or_skip("grid markers") else {
            return;
        };
        let markers = ['\u{25B2}', '\u{25BC}']; // ▲ ▼ — E22's sort indicator, in `Layout::header`
        let codepoints: Vec<u32> = markers.iter().map(|c| *c as u32).collect();
        let ids = p.grid_face().glyph_indices(&codepoints);
        let missing: Vec<char> = markers
            .iter()
            .zip(&ids)
            .filter(|(_, id)| **id == 0)
            .map(|(c, _)| *c)
            .collect();
        assert!(
            missing.is_empty(),
            "the grid face draws the sort indicator as .notdef boxes: {missing:?}"
        );
    }

    #[test]
    fn the_chrome_face_has_the_markers_the_chrome_draws() {
        let Some((_off, p)) = painter_or_skip("chrome markers") else {
            return;
        };
        // Every marker `draw_chrome` and the overlays actually draw. Segoe UI Variable Text has
        // `►` and `▼` but **not** `▸`, `▾` or `▶` — a distinction nothing but this test or the
        // window itself will tell you, and the first draft picked two of the missing three.
        let markers = [
            '\u{25BA}', // ► the find field's prompt
            '\u{25BC}', // ▼ the filter row, and the format chip's dropdown
            '\u{00D7}', // × a chip's remove
            '\u{25CF}', // ● an enabled highlight rule, and §2.2's ticked menu item
            '\u{25CB}', // ○ a disabled one
            // §2.2's column header will want a sort indicator, so the candidates were checked here
            // **before** anything drew them. `▲` is present and pairs with the `▼` above.
            //
            // **The small variants are not.** `▴` U+25B4 and `▾` U+25BE are both `.notdef` in
            // Segoe UI Variable Text, which is not guessable from `▲` and `▼` being present — they
            // are separate glyphs, exactly as `✓` was. Anyone reaching for a quieter arrow than
            // `▲` has to reach for a smaller *size*, not a smaller *character*.
            '\u{25B2}', // ▲ sort ascending, with U+25BC above as descending
        ];
        let codepoints: Vec<u32> = markers.iter().map(|c| *c as u32).collect();
        let ids = p.chrome_face().glyph_indices(&codepoints);
        let missing: Vec<char> = markers
            .iter()
            .zip(&ids)
            .filter(|(_, id)| **id == 0)
            .map(|(c, _)| *c)
            .collect();
        assert!(
            missing.is_empty(),
            "the chrome face draws these as .notdef boxes: {missing:?}"
        );
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
            .lay_out_row(
                &view,
                line,
                ColumnAnchors::none_ref(),
                Colours::plain(INK),
                0.0,
            )
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
            .lay_out_row(
                &view,
                &line,
                ColumnAnchors::none_ref(),
                Colours::plain(INK),
                0.0,
            )
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
                Colours::plain(INK),
                0.0,
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
                        Colours::plain(INK),
                        row as f32 * painter.row_height(),
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

    /// **A selected range reaches the quads as a filled background, and the ink is left alone.**
    ///
    /// The first shipped form re-tinted the glyphs instead — the owner read it as the font
    /// changing colour, which no standard application does. Selection is now a `MODE_SOLID` fill
    /// under the selected cells, emitted before the glyphs (buffer order is what makes a
    /// background a background), and the text keeps whatever ink it already had.
    ///
    /// The model half of selection was tested and passing while the feature was **invisible on
    /// screen**, because the shell handed the painter the wrong object and `row_selection` fell back
    /// to its default `None`. A test that calls `row_selection` directly cannot see that; this one
    /// goes through `lay_out` and inspects the instances, which is the path that was broken.
    #[test]
    fn a_selected_range_is_filled_behind_the_text_not_retinted() {
        let Some((_off, mut painter)) = painter_or_skip("a_selected_range_is_filled") else {
            return;
        };
        let view = view_for(&painter, 4, 200);

        // Rows 0 and 2 carry a selection; row 1 carries none.
        struct Selected(Vec<String>);
        impl RowSource for Selected {
            fn row_text(&self, row: u64) -> Option<&str> {
                self.0.get(usize::try_from(row).ok()?).map(String::as_str)
            }
            fn row_selection(&self, row: u64) -> Option<core::ops::Range<usize>> {
                (row != 1).then_some(0..4)
            }
        }
        let source = Selected(vec!["aaaaaaaa".to_owned(); 3]);

        painter.begin_frame();
        painter.lay_out(&view, INK, &source).expect("lay out");
        let instances = painter.instances();
        let fills: Vec<usize> = instances
            .iter()
            .enumerate()
            .filter(|(_, i)| i.mode == MODE_SOLID && i.tint == Theme::dark().selection_bg)
            .map(|(at, _)| at)
            .collect();
        let cell_w = view.hgrid().cell_width();
        for (_, fill) in instances
            .iter()
            .enumerate()
            .filter(|(at, _)| fills.contains(at))
        {
            assert_eq!(
                fill.size[0],
                4.0 * cell_w,
                "each fill covers the four selected columns"
            );
        }
        let retinted = instances
            .iter()
            .filter(|i| i.mode != MODE_SOLID && i.tint != INK)
            .count();

        assert_eq!(fills.len(), 2, "one fill on each of the two selected rows");
        // Buffer order is the entire mechanism by which a background is a background: each row's
        // fill must be emitted before that row's own glyphs, or it paints over them.
        let row_h = view.grid().row_height();
        for &at in &fills {
            let fill_y = instances[at].pos[1];
            let overdrawn = instances[..at]
                .iter()
                .filter(|i| i.mode != MODE_SOLID)
                .filter(|i| i.pos[1] >= fill_y && i.pos[1] < fill_y + row_h)
                .count();
            assert_eq!(
                overdrawn, 0,
                "a selection fill was emitted after glyphs on its own row"
            );
        }
        assert_eq!(retinted, 0, "selected text keeps the ink it already had");
    }

    /// **§7.1's colours, from a span to the quads that draw it.**
    ///
    /// Three claims in one test, because they only mean anything together: a background covers the
    /// span's columns and no others, it is emitted **before** the glyphs it sits behind — buffer
    /// order is the entire mechanism by which a background is a background — and the foreground
    /// re-tints exactly the span's clusters.
    ///
    /// The ordering claim is the one that would otherwise be untested and is the easiest to break:
    /// a background appended at the end of the row draws *over* the text, which looks like the
    /// highlight working until you notice the log line has gone.
    /// §13.4's reveal toggle: a bidi override — the Trojan Source character — draws nothing with
    /// the toggle off and occupies no column; with it on it takes a column and that column holds a
    /// marker quad rather than the override's own glyph, and the character after it moves right by
    /// one cell. `a` + ZWJ, absorbed into `a`'s cluster, is the recorded gap and stays unrevealed.
    #[test]
    fn a_revealed_invisible_draws_a_marker_in_its_own_column() {
        let Some((off, mut painter)) = painter_or_skip("a_revealed_invisible_draws_a_marker")
        else {
            return;
        };
        let mut view = view_for(&painter, 1, 200);
        let line = "ab\u{202E}cd";

        painter.begin_frame();
        painter
            .lay_out_row(
                &view,
                line,
                ColumnAnchors::none_ref(),
                Colours::plain(INK),
                0.0,
            )
            .expect("lay out");
        let solids = painter
            .instances()
            .iter()
            .filter(|i| i.mode == MODE_SOLID)
            .count();
        assert_eq!(solids, 0, "off: nothing marks the override");
        let glyphs_off = painter.instances().len();

        view.cells_mut().reveal_invisibles = true;
        painter.begin_frame();
        painter
            .lay_out_row(
                &view,
                line,
                ColumnAnchors::none_ref(),
                Colours::plain(INK),
                0.0,
            )
            .expect("lay out");
        let solids: Vec<&Instance> = painter
            .instances()
            .iter()
            .filter(|i| i.mode == MODE_SOLID)
            .collect();
        assert_eq!(solids.len(), 1, "on: one marker");
        assert_eq!(solids[0].tint, Theme::dark().reveal_mark);
        let col2 = view.hgrid().x_of_column(2);
        assert!(
            solids[0].pos[0] > col2
                && solids[0].pos[0] + solids[0].size[0] < col2 + painter.cell_width(),
            "the marker sits inside column 2: {:?}",
            solids[0]
        );
        assert_eq!(
            painter.instances().len(),
            glyphs_off + 1,
            "the override still shapes to no drawn glyph; only the marker was added"
        );
        drop(off);
    }

    #[test]
    fn a_spans_background_is_drawn_under_exactly_its_own_columns() {
        let Some((off, mut painter)) = painter_or_skip("a_spans_background_is_drawn_under") else {
            return;
        };
        let view = view_for(&painter, 1, 200);
        let line = "aaaaBBBBaaaa";
        const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
        const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
        let spans = [Span {
            start: 4,
            end: 8,
            fg: Some(RED),
            bg: Some(GREEN),
        }];

        painter.begin_frame();
        painter
            .lay_out_row(
                &view,
                line,
                ColumnAnchors::none_ref(),
                Colours {
                    spans: &spans,
                    ..Colours::plain(INK)
                },
                0.0,
            )
            .expect("lay out");

        let solids: Vec<&Instance> = painter
            .instances()
            .iter()
            .filter(|i| i.mode == MODE_SOLID)
            .collect();
        assert_eq!(solids.len(), 1, "one span with a background is one quad");
        assert!(
            (solids[0].pos[0] - view.hgrid().x_of_column(4)).abs() < 0.01,
            "the background starts at {} rather than column 4's x of {}",
            solids[0].pos[0],
            view.hgrid().x_of_column(4)
        );
        assert!(
            (solids[0].size[0] - 4.0 * painter.cell_width()).abs() < 0.01,
            "the background is {} wide rather than four columns",
            solids[0].size[0]
        );
        assert_eq!(solids[0].size[1], painter.row_height());
        assert_eq!(
            painter.instances()[0].mode,
            MODE_SOLID,
            "the background must be the row's first instance or it draws over the text"
        );

        let tinted = |want: [f32; 4]| {
            painter
                .instances()
                .iter()
                .filter(|i| i.mode != MODE_SOLID && i.tint == want)
                .count()
        };
        assert_eq!(tinted(RED), 4, "exactly the span's four clusters re-tint");
        assert_eq!(tinted(INK), 8, "the rest of the line keeps the plain ink");

        // And it reaches pixels, which no instance assertion can show: a quad of the right colour
        // in the right place still draws nothing if its mode is wrong.
        //
        // **Every green pixel is counted, rather than one being sampled**, because a sampled point
        // can land on the glyph ink drawn over the background and fail for the one reason the test
        // is not about. The bounds are the assertion: green inside the span's columns, nowhere else.
        off.clear(PAPER);
        painter.draw(off.context(), (TARGET, TARGET)).expect("draw");
        let pixels = off.read_back().expect("read back");
        let (mut green, mut leftmost, mut rightmost) = (0u32, TARGET, 0u32);
        for y in 0..painter.row_height() as u32 {
            for x in 0..TARGET {
                let p = pixels.at(x, y);
                if p[1] > p[0] + 20 && p[1] > p[2] + 20 {
                    green += 1;
                    leftmost = leftmost.min(x);
                    rightmost = rightmost.max(x);
                }
            }
        }
        assert!(green > 0, "the span's background reached no pixels at all");
        let (from, to) = (
            view.hgrid().x_of_column(4) as u32,
            view.hgrid().x_of_column(8) as u32,
        );
        assert!(
            leftmost >= from && rightmost < to,
            "the background painted x {leftmost}..={rightmost} against the span's {from}..{to}"
        );
    }

    /// A span the horizontal scroll has moved off screen must not drag its background back on.
    ///
    /// `byte_span` rounds the visible slice outwards to whole clusters, so clipping the span to the
    /// slice is what keeps a background inside the viewport — and a whole-line rule (§7.1) names
    /// bytes off both edges of every scrolled row, so this is the ordinary case rather than an edge
    /// one.
    #[test]
    fn a_background_is_clipped_to_the_visible_slice() {
        let Some(off) = painter_or_skip("a_background_is_clipped_to_the_visible_slice") else {
            return;
        };
        let (off, mut painter) = off;
        let mut view = view_for(&painter, 1, 200);
        let line = "0123456789abcdefghijklmnopqrstuvwxyz";
        let whole_line = [Span {
            start: 0,
            end: line.len(),
            fg: None,
            bg: Some([0.0, 1.0, 0.0, 1.0]),
        }];

        view.hgrid_mut().scroll_to_column(10);
        painter.begin_frame();
        painter
            .lay_out_row(
                &view,
                line,
                ColumnAnchors::none_ref(),
                Colours {
                    spans: &whole_line,
                    ..Colours::plain(INK)
                },
                0.0,
            )
            .expect("lay out");

        let solid = painter
            .instances()
            .iter()
            .find(|i| i.mode == MODE_SOLID)
            .expect("a whole-line background");
        assert!(
            solid.pos[0] >= -0.01,
            "the background starts at {}, left of the viewport",
            solid.pos[0]
        );
        // **One cell of slack on the right, and it is `byte_span`'s outward rounding rather than
        // laxity.** The slice ends on a whole cluster, so the column straddling the right edge is
        // drawn — and a background that stopped short of the glyph over it would be the visible
        // half of the same off-by-one. What is asserted is that it stops *there* and not at the
        // line's end, which is 36 columns away.
        let right = solid.pos[0] + solid.size[0];
        let limit = view.hgrid().viewport_px() + painter.cell_width();
        assert!(
            right <= limit + 0.01,
            "the background reaches x {right} against a {} viewport plus one cell",
            view.hgrid().viewport_px()
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
            .lay_out_row(
                &view,
                line,
                ColumnAnchors::none_ref(),
                Colours::plain(INK),
                0.0,
            )
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
                .lay_out_row(
                    &view,
                    line,
                    ColumnAnchors::none_ref(),
                    Colours::plain(INK),
                    0.0,
                )
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
