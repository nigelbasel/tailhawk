//! Tailhawk core — the portable half of the seam in `SPEC.md` §3.1.
//!
//! The core owns the whole grid *including rendering*; the shell owns window, input and IME and
//! hands the core a drawable. At M0 there is no grid yet, so what is here is the seam itself and
//! the presentation leaf backend.
//!
//! Nothing in this crate may reference an `HWND`, a message loop, or any other shell concept. The
//! drawable crosses the seam as an opaque [`WindowHandle`].

#![deny(unsafe_op_in_unsafe_fn)]

pub mod ansi;
pub mod atlas;
pub mod bidi;
pub mod cell;
pub mod columns;
pub mod detail;
pub mod detect;
pub mod encoding;
pub mod filter;
pub mod follow;
pub mod format;
pub mod grid;
pub mod hgrid;
pub mod highlight;
pub mod index;
pub mod indexer;
pub mod lines;
pub mod palette;
pub mod pattern;
pub mod record;
pub mod rows;
pub mod search;
pub mod selection;
pub mod semantic;
pub mod settings;
pub mod template;
pub mod theme;
pub mod view;
pub mod widget;

#[cfg(windows)]
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext};

#[cfg(windows)]
pub mod file;

/// A search on a worker. Windows-only because it holds the file handles [`file`] owns.
#[cfg(windows)]
pub mod find;

/// The filter pass on a worker. Windows-only for the same reason as [`find`].
#[cfg(windows)]
pub mod sieve;

/// Export and tee on a worker. Windows-only for the same reason as [`find`].
#[cfg(windows)]
pub mod export;

#[cfg(windows)]
pub mod glyphs;

#[cfg(windows)]
pub mod paint;

#[cfg(windows)]
pub mod raster;

#[cfg(windows)]
pub mod rotation;

#[cfg(windows)]
pub mod scanner;
pub mod set;

#[cfg(windows)]
pub mod stdin;

#[cfg(windows)]
pub mod shape;

#[cfg(windows)]
pub mod sheet;

#[cfg(windows)]
pub mod text;

#[cfg(windows)]
mod gpu;

#[cfg(windows)]
pub use gpu::Driver;

#[cfg(windows)]
pub use file::{FileError, FileIdentity, FileSource, LogFile};

#[cfg(windows)]
pub use rotation::{Rotation, Watch};

pub use atlas::{
    Atlas, FaceId, GlyphId, GlyphKey, Ink, InsertError, Placement, Residency, SlotId, Synthetic,
};
pub use bidi::{reorder, visual_order};
pub use cell::{Cell, CellModel};
pub use encoding::{detect, Charset, Confidence, Detection, Sample};
pub use follow::{Follow, Poll, FOLLOW_BUDGET_BYTES};
#[cfg(any(test, feature = "test-hooks"))]
pub use gpu::offscreen::Pixels;
pub use grid::{Grid, PlacedRow, Scroll};
pub use hgrid::{HGrid, MAX_CELL_WIDTH_PX, RENDER_CAP_CELLS};
pub use index::{Anchor, LineIndex, LineScanner, ANCHOR_STRIDE};
pub use indexer::{build_index, offset_of_line, ChunkReader, IndexOptions};
pub use lines::LineDecoder;
pub use rows::{RowSource, Rows};
pub use selection::{Position, RowEnd, RowSpan, Selection, SelectionMode};
pub use view::{RowSlice, View};

pub use record::{
    AttributeValue, FormatId, ParseState, Record, Resource, Severity, SeverityBand, Timestamp,
    TraceContext, ERROR_THRESHOLD,
};
#[cfg(windows)]
pub use shape::{ClusterGlyphs, ClusterSpan, GlyphOffset, Shaped, Shaper};

/// An opaque platform window handle. On Windows this is an `HWND`; the core never interprets it,
/// it only passes it to the presentation backend.
///
/// This exists so that `SPEC.md` §3.1's rule — the core is portable, the shell is not — is
/// enforced by the type system rather than by discipline.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct WindowHandle(pub isize);

/// The window background, in linear-ish sRGB values for the render target.
///
/// **Both stages of the two-stage first paint must use this exact colour** (`SPEC.md` §3.2): the
/// shell fills the client area with it before a device exists, the renderer clears to it once the
/// device is up, and the transition is invisible only if they agree. That is why it lives in the
/// core rather than in the shell — the shell borrows it, so the two cannot drift apart.
///
/// Provisional: `UI-DESIGN.md` §10 describes the palette qualitatively and pins no hex value yet.
pub const BACKGROUND: [f32; 4] = [0.071, 0.078, 0.090, 1.0];

/// The same colour as 8-bit sRGB, for the shell's GDI stage.
pub const fn background_rgb8() -> (u8, u8, u8) {
    (18, 20, 23)
}

/// Faces to resolve the grid's font from, most monospace-appropriate first.
///
/// All four ship with Windows and the list degrades rather than fails: Cascadia Mono is the modern
/// terminal face, Consolas has been in the box since Vista, Courier New since forever, and Segoe UI
/// is the proportional last resort that keeps a face resolvable on a stripped install. §3.1's cell
/// grid tolerates a proportional face — it measures one and uses it — so the fallback degrades the
/// *look* and not the correctness.
pub const DEFAULT_FONTS: &[&str] = &["Cascadia Mono", "Consolas", "Courier New", "Segoe UI"];

/// The em size at 100%, in device pixels. [`Renderer::set_dpi`] scales it per monitor, so this is
/// the value at the Win32 unit-DPI baseline of 96 and not a fixed size.
pub const DEFAULT_PX_PER_EM: u16 = 16;

/// The grid's foreground. Provisional alongside [`BACKGROUND`] — `UI-DESIGN.md` §10 pins no hex.
pub const INK: [f32; 4] = [0.878, 0.890, 0.906, 1.0];

/// Selected text, until there is a background pass to highlight it properly.
///
/// **⚠ This is a stand-in for a highlight, not a colour choice**, and the reason it used to give —
/// that `Instance` has no background field — is no longer the reason. `text.rs`'s `MODE_SOLID` and
/// `paint.rs`'s background pass mean a filled selection is now available for the asking; what is
/// missing is the decision about what colour it should be, which `UI-DESIGN.md` §10 leaves open
/// along with the rest of the palette. Re-tinting consumes exactly the per-row column range a fill
/// would, so nothing is thrown away when that decision is taken.
pub const SELECTION_INK: [f32; 4] = [0.45, 0.72, 1.0, 1.0];

/// The background behind every match of the running search.
///
/// **Muted, because every hit in the file wears it.** A screenful of saturated highlight is a
/// screenful nobody can read, and §7.4 streams up to 100,000 matches into a file whose lines are
/// mostly still ordinary text.
pub const MATCH_BG: [f32; 4] = [0.36, 0.29, 0.06, 1.0];

/// The background behind the **current** match — the one `F3` last stepped to.
///
/// **The distinction is the whole of stepping.** Every match looking identical makes `F3` a
/// scroll with no destination: the view moves and nothing says which of the four hits on screen is
/// the one it moved to.
pub const CURRENT_MATCH_BG: [f32; 4] = [0.95, 0.62, 0.16, 1.0];

/// Ink over [`CURRENT_MATCH_BG`], which is bright enough that [`INK`] would not be legible on it.
///
/// ⚠ Provisional with the rest of the palette, and **contrast here is a correctness question rather
/// than a taste one** — `SPEC.md` §14 wants a High Contrast path and a non-colour channel, and
/// neither exists yet. Two colours a user cannot tell apart is the failure this pairing is one
/// unverified guess away from.
pub const CURRENT_MATCH_INK: [f32; 4] = [0.071, 0.078, 0.090, 1.0];

/// The marker drawn in the cell of a revealed invisible — §13.4's toggle. A muted amber block,
/// unlike any glyph, so a hidden character reads as "something is here" and never as text.
/// Provisional with the rest of the palette.
pub const REVEAL_MARK: [f32; 4] = [0.85, 0.65, 0.25, 1.0];

/// The ink of a continuation line — a stack-trace frame under its record's first line, §6.4:
/// "rendered dimmed and indented". A step under [`INK`], not a colour of its own.
pub const CONTINUATION_INK: [f32; 4] = [0.56, 0.59, 0.64, 1.0];

/// The column header band and its text — a shade above the ground, the names in the muted ink, so
/// the header reads as chrome and never as a log line. Provisional with the palette.
pub const HEADER_BG: [f32; 4] = [0.11, 0.12, 0.14, 1.0];
pub const HEADER_INK: [f32; 4] = [0.62, 0.66, 0.72, 1.0];

/// The gutter's line numbers: quieter than the header, present on every row.
pub const GUTTER_INK: [f32; 4] = [0.40, 0.44, 0.50, 1.0];

/// Errors that cross the seam. Deliberately opaque — the shell can report one but cannot act on
/// the distinction, and `SPEC.md` §3.2 forbids panicking on device loss.
#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Builds the text resources if the slot is empty or was built for a **different device or a
/// different scale**, and hands back whatever is now current.
///
/// The staleness test is a comparison rather than a flag, because a flag has to be remembered at
/// every site that could invalidate it and a comparison cannot be forgotten. The cost of getting the
/// *device* half wrong is not an error: a `Sheet` from a released device draws **nothing**, so a
/// renderer that recovered from device loss would return `Ok` for ever while the screen stayed
/// blank — the outcome `SPEC.md` §3.2's recovery exists to prevent.
///
/// **The scale is the second half of the key, and §3.1 requires it.** The atlas holds glyphs
/// rasterised at one `px_per_em`; §3.2 keys it on `(glyph id, style, dpi scale)` and §3.1 says it is
/// "rebuilt per scale factor". Dragging a window from a 100% to a 150% monitor without rebuilding
/// would draw 16 px rasters into 24 px cells — text that is both blurry and out of column, which is
/// the exact drift §3.1's integer-advance rule exists to prevent.
///
/// Taking the device rather than the `Gpu` is what lets the **in-frame** caller use it: during
/// `render_frame_with` the `Gpu` is mutably borrowed and cannot lend its device out, but the draw
/// callback is handed one — and that path is the one that matters, because it is the retry after a
/// mid-frame rebuild.
#[cfg(windows)]
fn ensure_painter<'a>(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    generation: u32,
    slot: &'a mut Option<(u32, u16, paint::Painter)>,
    candidates: &[String],
    px_per_em: u16,
) -> Result<&'a mut paint::Painter> {
    if !matches!(slot, Some((g, px, _)) if *g == generation && *px == px_per_em) {
        // Released before the replacement is asked for, so a driver mid-reset is not holding two
        // atlases at once.
        *slot = None;
        let names: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let mut fresh = paint::Painter::new(device, &names, px_per_em)?;
        fresh.prime(context);
        *slot = Some((generation, px_per_em, fresh));
    }
    Ok(&mut slot.as_mut().expect("just built").2)
}

/// The renderer the shell drives.
///
/// Construction creates the graphics device and is the expensive step — `experiments/g3-d3d11`
/// measured it as the dominant cost of first paint — so the shell is expected to call
/// [`Renderer::new`] on a worker thread while the window comes up on the main thread.
#[cfg(windows)]
pub struct Renderer {
    gpu: gpu::Gpu,
    /// The text pass, and **the device generation it was built against**.
    ///
    /// Not an invalidation flag, because a flag can be forgotten to be set. A `Painter` owns a
    /// `GlyphCache`, which owns a `Sheet` — a texture belonging to one `ID3D11Device`. When
    /// `gpu.rs` replaces a lost device, drawing from the old sheet does not fail; it silently
    /// produces **nothing**, which is the exact failure §3.2's recovery exists to prevent and the
    /// hardest kind to notice. Storing the generation alongside makes staleness a comparison that
    /// [`ensure_painter`] cannot skip.
    ///
    /// The `u16` is the `px_per_em` it was rasterised at — the second half of the key, per §3.1's
    /// "the glyph atlas is rebuilt per scale factor".
    painter: Option<(u32, u16, paint::Painter)>,
    font_candidates: Vec<String>,
    px_per_em: u16,
}

#[cfg(windows)]
impl Renderer {
    /// Creates the graphics device, walking `SPEC.md` §3.2's fallback chain.
    ///
    /// **Call this off the thread that owns the window.** Beyond the ~8.5 ms it saves on an idle
    /// machine, `experiments/g3-d3d11` found serial device creation degrading past a second under
    /// GPU-context pressure while the off-thread path held at ~135 ms.
    pub fn new() -> Result<Self> {
        Ok(Self {
            gpu: gpu::Gpu::new()?,
            painter: None,
            font_candidates: DEFAULT_FONTS.iter().map(|s| (*s).to_owned()).collect(),
            px_per_em: DEFAULT_PX_PER_EM,
        })
    }

    /// Sets the scale from a monitor DPI, and returns whether it changed.
    ///
    /// **This is the whole of §3.1's "re-derived on any scale change".** The em size is rounded to a
    /// whole device pixel here rather than carried as a float, because everything downstream —
    /// rasterisation, the cell box, the column advance — is integer device pixels at the current
    /// scale, and §3.1 is explicit that fractional per-glyph rounding "accumulates drift and visibly
    /// misaligns columns across a wide window".
    ///
    /// A changed scale does **not** rebuild anything here. It cannot: the rebuild needs a device,
    /// and at a `WM_DPICHANGED` the caller is usually about to resize too. [`ensure_painter`] sees
    /// the new value on the next frame and rebuilds the atlas then, which is also what keeps the
    /// device-loss and scale-change paths from being two mechanisms.
    pub fn set_dpi(&mut self, dpi: u32) -> bool {
        // 96 is the Win32 unit-DPI baseline, so `dpi / 96` is the scale factor.
        let scaled = (f64::from(DEFAULT_PX_PER_EM) * f64::from(dpi.max(1)) / 96.0).round();
        // A face is unusable below a pixel or two, and DirectWrite is entitled to refuse absurd
        // sizes; clamping keeps a hostile or bogus DPI from turning a scale change into an error.
        let px = scaled.clamp(6.0, 400.0) as u16;
        let changed = px != self.px_per_em;
        self.px_per_em = px;
        changed
    }

    /// The em size the grid is currently rasterised at, in device pixels.
    pub fn px_per_em(&self) -> u16 {
        self.px_per_em
    }

    /// The cell the current face measures to, as `(width, height)` in device pixels.
    ///
    /// **This is what [`View::set_metrics`](view::View::set_metrics) must be fed**, per §3.1's
    /// integer cell advances re-derived from the face at the current scale — a constant would drift
    /// from the font the moment the DPI changes.
    ///
    /// It builds the text resources if they are not up yet, because the alternative is a
    /// chicken-and-egg the caller cannot break: a `View` needs the cell before it can say which rows
    /// are visible, and the painter that measures the cell would otherwise only exist once a frame
    /// has been drawn. Re-read it after a resize or a DPI change rather than caching it.
    pub fn cell(&mut self) -> Result<(f32, f32)> {
        let Self {
            gpu,
            painter,
            font_candidates,
            px_per_em,
        } = self;
        let (device, context) = gpu.resources();
        let p = ensure_painter(
            device,
            context,
            gpu.generation(),
            painter,
            font_candidates,
            *px_per_em,
        )?;
        Ok((p.cell_width(), p.row_height()))
    }

    /// Which rung of the fallback chain the device is *currently* on.
    ///
    /// This can change while the renderer is running: `SPEC.md` §3.2's device-removed recovery
    /// drops to WARP when a device will not stay up, so a caller that shows this to the user
    /// should re-read it rather than cache it.
    pub fn driver(&self) -> Driver {
        self.gpu.driver()
    }

    /// How many graphics devices this renderer has built, starting at 1. It rises when a lost
    /// device is replaced, which is otherwise an entirely invisible event.
    pub fn device_generation(&self) -> u32 {
        self.gpu.generation()
    }

    /// Binds the renderer to a window. Idempotent.
    pub fn attach(&mut self, window: WindowHandle, width: u32, height: u32) -> Result<()> {
        self.gpu.attach(window, width, height)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.gpu.resize(width, height)
    }

    /// Draws one frame and presents it. At M0 this is the background clear and nothing else —
    /// the grid arrives at M3.
    ///
    /// A device lost between frames — a driver auto-update, a TDR, a GPU switch — is rebuilt
    /// here and the frame is redrawn, so the caller sees `Ok` and never learns it happened.
    /// `SPEC.md` §3.2 forbids panicking on device-removed; an `Err` here means the device could
    /// not be rebuilt after several attempts, not that the process should end.
    pub fn paint(&mut self) -> Result<()> {
        self.gpu.render_frame(theme::theme().background)
    }

    /// Draws one frame of text over the background and presents it.
    ///
    /// `line_at` returns a row's text, or `None` for a row not in memory yet — which draws nothing
    /// rather than blocking, per §11.3. The returned [`Laid`](paint::Laid) carries the counts the
    /// text pass is obliged to disclose: glyphs queued for rasterisation, rows truncated, rows that
    /// failed to shape, and RTL runs it could not place.
    ///
    /// **The painter is rebuilt inside the frame, not before it.** A device lost *during* this call
    /// is replaced by `gpu.rs` and the frame is redrawn against the new device — at which point the
    /// glyph atlas from the old one is a texture no longer bound to anything, and drawing from it
    /// produces nothing at all rather than an error. So the rebuild check lives in the draw callback
    /// where it sees the generation of whichever device is actually current, and it runs on the
    /// retry as well as the first attempt.
    ///
    /// **⚠ Rasterisation still has no home on this path.** `flush_misses` must run *after*
    /// presenting, per §3.2 and `experiments/g4b-batched-raster`'s 162 ms cold viewport, and nothing
    /// calls it yet — so a cold frame draws placeholder boxes and the next frame draws them again.
    /// Wiring it needs a post-present hook the shell drives; that is the next M3 step, not this one.
    #[cfg(any(test, feature = "test-hooks"))]
    /// One frame of `source` rendered to an offscreen target of `width × height` and read back —
    /// a headless screenshot, for a harness that has no desktop to capture. Two frames are drawn,
    /// because the first of a cold atlas is placeholders (§3.2) and the misses are flushed between.
    pub fn snapshot(
        &mut self,
        width: u32,
        height: u32,
        view: &view::View,
        source: &dyn rows::RowSource,
    ) -> Result<gpu::offscreen::Pixels> {
        self.snapshot_panes(width, height, &[(view, source, 0.0)])
    }

    /// [`Renderer::snapshot`] for a split: several panes at their offsets.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn snapshot_panes(
        &mut self,
        width: u32,
        height: u32,
        panes: &[(&view::View, &dyn rows::RowSource, f32)],
    ) -> Result<gpu::offscreen::Pixels> {
        self.gpu.attach_offscreen(width, height)?;
        for _ in 0..2 {
            self.gpu.clear_offscreen(theme::theme().background);
            self.paint_panes(panes, (width as f32, height as f32))?;
        }
        self.gpu.read_back()
    }

    pub fn paint_rows(
        &mut self,
        view: &view::View,
        source: &dyn rows::RowSource,
    ) -> Result<paint::Laid> {
        self.paint_panes(&[(view, source, 0.0)], (view.gutter_px() + view.hgrid().viewport_px(), view.height_px()))
    }

    /// Several panes in one frame — a split view. Each `(view, source, y)` is laid out as if it
    /// were the whole client and then moved down by `y`; `client` is the client size the shader
    /// maps pixels by. Panes are drawn in order, so a later pane's bands cover an earlier pane's
    /// last partial row.
    pub fn paint_panes(
        &mut self,
        panes: &[(&view::View, &dyn rows::RowSource, f32)],
        client: (f32, f32),
    ) -> Result<paint::Laid> {
        // Disjoint field borrows: the callback needs `painter` mutably while `gpu` is borrowed for
        // the frame. Destructuring is what makes that legal, and it is also what forces the painter
        // to be reachable from inside the retry.
        let Self {
            gpu,
            painter,
            font_candidates,
            px_per_em,
        } = self;
        let mut laid = paint::Laid::default();

        let t = theme::theme();
        gpu.render_frame_with(t.background, &mut |device, context, generation| {
            let p = ensure_painter(
                device,
                context,
                generation,
                painter,
                font_candidates,
                *px_per_em,
            )?;

            p.begin_frame();
            laid = paint::Laid::default();
            for (view, source, y) in panes {
                let from = p.mark();
                laid.merge(p.lay_out(view, t.ink, *source)?);
                p.shift(from, 0.0, *y);
            }
            // The whole client, gutter included, taken from the caller rather than the swapchain
            // so `gpu` stays out of this closure — which is what makes the disjoint borrow above
            // hold. The shader maps pixels to clip space by this number, so a width short of the
            // gutter would draw every frame stretched by the gutter's share — which it did, for
            // two commits.
            p.draw(
                context,
                (client.0.max(1.0) as u32, client.1.max(1.0) as u32),
            )
        })?;

        // **After presenting, which is what `render_frame_with` returning means.** §3.2 requires
        // the ordering and `experiments/g4b-batched-raster` measured why: a genuinely cold
        // 1,500-glyph viewport costs 162 ms, eight to ten frames' worth, and paying it before the
        // present would stall every one of them. Paying it *nowhere* is not the alternative it might
        // look like — the atlas would never fill, so every frame would draw placeholder boxes for
        // ever and `queued` would never fall to zero.
        if let Some((generation, _, p)) = painter.as_mut() {
            debug_assert_eq!(*generation, gpu.generation());
            let (_, context) = gpu.resources();
            laid.rasterised = p.flush_misses(context)?;
        }
        Ok(laid)
    }

    /// Makes the next [`paint`](Self::paint) see the device as removed, so the recovery path runs
    /// for real against a real device.
    ///
    /// D3D11 has no API for removing a device — unlike D3D12's `RemoveDevice` — and the genuine
    /// causes are a driver update or a TDR, neither of which a test can arrange. Everything after
    /// the injected `DXGI_ERROR_DEVICE_REMOVED` is the real path: the same classification, the
    /// same policy, a real device rebuild and a real `Present`.
    #[cfg(feature = "test-hooks")]
    pub fn simulate_device_loss(&mut self) {
        self.gpu.simulate_device_loss();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A [`rows::RowSource`] over a plain list, with no anchors.
    #[cfg(windows)]
    struct Listed(Vec<String>);

    #[cfg(windows)]
    impl rows::RowSource for Listed {
        fn row_text(&self, row: u64) -> Option<&str> {
            self.0.get(usize::try_from(row).ok()?).map(String::as_str)
        }
    }

    /// A renderer and a view sized to it, or `None` on a machine with no usable device.
    #[cfg(windows)]
    fn renderer_or_skip(what: &str) -> Option<(Renderer, view::View)> {
        let mut r = match Renderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipping {what}: no graphics device ({e})");
                return None;
            }
        };
        let (cw, rh) = r.cell().expect("the face measures");
        let mut v = view::View::new(cw, rh);
        v.set_viewport(80.0 * cw, 8.0 * rh);
        v.grid_mut().set_total_rows(8);
        v.hgrid_mut().set_columns(80);
        Some((r, v))
    }

    /// The text resources are keyed to the device that built them, and a rebuild replaces them.
    ///
    /// **⚠ What this test can and cannot prove, stated rather than implied.** It proves the painter
    /// is re-created for the new generation, which is the mechanism. It does **not** prove the
    /// pixels, because a `Renderer` with no window holds no render target and there is nothing to
    /// read back — `paint.rs`'s `a_viewport_of_rows_reaches_real_pixels` covers that against an
    /// `Offscreen`. The gap matters because the failure being guarded is *silent*: a `Sheet` from a
    /// released device draws nothing and reports success, so a stale painter and a correct one are
    /// indistinguishable from the return value alone. The generation identity is the only thing a
    /// headless test can check, and it is checked directly rather than inferred.
    #[cfg(all(windows, feature = "test-hooks"))]
    #[test]
    fn a_rebuilt_device_gets_rebuilt_text_resources() {
        let Some((mut r, view)) = renderer_or_skip("a_rebuilt_device_gets_rebuilt_text_resources")
        else {
            return;
        };
        let line = Listed(
            (0..8)
                .map(|r| format!("row {r} — the quick brown fox"))
                .collect(),
        );

        let first = r.paint_rows(&view, &line).expect("the first text frame");
        assert!(first.quads > 0, "nothing was laid out");
        assert_eq!(r.device_generation(), 1);
        assert_eq!(
            r.painter.as_ref().map(|(g, _, _)| *g),
            Some(1),
            "the painter should be keyed to the device that built it"
        );

        r.simulate_device_loss();
        let after = r
            .paint_rows(&view, &line)
            .expect("a text frame across a device loss still presents");

        assert_eq!(r.device_generation(), 2, "the device was not rebuilt");
        assert_eq!(
            r.painter.as_ref().map(|(g, _, _)| *g),
            Some(2),
            "the painter is still keyed to the dead device — its atlas draws nothing and says nothing"
        );
        assert!(after.quads > 0, "the frame after recovery laid nothing out");
        // The new atlas starts empty, so the recovered frame queues its glyphs again and pays for
        // them after presenting. That it is non-zero is the evidence the cache really was replaced.
        assert!(
            after.queued > 0,
            "the recovered frame queued nothing, so the old atlas was probably reused"
        );
    }

    /// A scale change re-keys the atlas, and the cell grows with it.
    ///
    /// §3.1: "the glyph atlas is rebuilt per scale factor" and "column advances are computed in
    /// integer device pixels at the current scale and **re-derived on any scale change**". Keeping
    /// the 100% atlas across a drag to a 150% monitor draws 16 px rasters into 24 px cells — blurry
    /// *and* progressively out of column, which is the drift the integer-advance rule exists to
    /// prevent. The painter identity is checked directly for the same reason as the device-loss
    /// test: a stale atlas still draws, so the return value alone cannot tell you.
    #[cfg(windows)]
    #[test]
    fn a_scale_change_rebuilds_the_atlas_and_regrows_the_cell() {
        let Some((mut r, view)) = renderer_or_skip("a_scale_change_rebuilds_the_atlas") else {
            return;
        };
        let line = Listed(vec!["the quick brown fox".to_owned(); 8]);

        assert_eq!(r.px_per_em(), DEFAULT_PX_PER_EM);
        let at_100 = r.cell().expect("a cell at 100%");
        r.paint_rows(&view, &line).expect("a frame at 100%");
        assert_eq!(r.painter.as_ref().map(|(_, px, _)| *px), Some(16));

        // 144 dpi is the 150% monitor named in M3's done-criterion.
        assert!(r.set_dpi(144), "150% should be a scale change");
        assert_eq!(r.px_per_em(), 24, "16 px at 96 dpi is 24 px at 144");

        let at_150 = r.cell().expect("a cell at 150%");
        assert!(
            at_150.0 > at_100.0 && at_150.1 > at_100.1,
            "the cell did not grow: {at_100:?} then {at_150:?}"
        );
        // Integer device pixels, per §3.1 — a fractional advance drifts across a wide window.
        assert_eq!(at_150.0.fract(), 0.0);
        assert_eq!(at_150.1.fract(), 0.0);

        r.paint_rows(&view, &line).expect("a frame at 150%");
        assert_eq!(
            r.painter.as_ref().map(|(_, px, _)| *px),
            Some(24),
            "the atlas is still the 100% one, so every glyph is being upscaled into a bigger cell"
        );

        // Idempotent: the same DPI twice is not a change, so it must not throw the atlas away.
        assert!(!r.set_dpi(144));
        // And an absurd DPI is clamped rather than passed to DirectWrite as a face size.
        assert!(r.set_dpi(1_000_000));
        assert!(r.px_per_em() <= 400);
        assert!(r.cell().is_ok(), "a clamped scale still measures");
    }

    /// **The background is still the background on the second frame.**
    ///
    /// This is the regression test for the worst bug of the session, and it is worth saying plainly
    /// why it did not exist at the time. `TextPipeline::draw` binds a dual-source blend state and
    /// does not restore it; `draw_background` set none at all. So frame 1 inherited D3D11's default
    /// and was correct, and **every frame after it left the target untouched** — for fifteen
    /// sessions, in M0 code, invisibly, because on a zeroed back buffer that reads as RGB(0,0,0)
    /// where RGB(18,20,23) belongs and no eye finds that. It was caught by sampling a screenshot,
    /// not by any test, and it could not have been: `draw_background` needs a render target, and
    /// the only way to get one was a swapchain, which needs a window.
    ///
    /// **Two frames, and a clear to magenta before each, and both are load-bearing.** The first
    /// version of this test had a control that did **not** fire: an offscreen target keeps the
    /// previous frame's pixels, so a background that was never drawn left the *correct* colour
    /// behind and read as success. A real swapchain does not do that — a fresh flip-model buffer is
    /// zeroed — so the test was passing for a reason the shipped code does not enjoy. Clearing to a
    /// colour no pass can produce restores the distinction. The control then fires with the corner
    /// still **magenta**, which also corrected the diagnosis: the background pass produces
    /// *nothing*, rather than producing black.
    #[cfg(windows)]
    #[test]
    fn the_background_survives_a_frame_of_text() {
        let mut r = match Renderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipping the_background_survives_a_frame_of_text: no device ({e})");
                return;
            }
        };
        r.gpu
            .attach_offscreen(256, 128)
            .expect("an offscreen target");

        let (cw, rh) = r.cell().expect("the face measures");
        let mut view = view::View::new(cw, rh);
        view.set_viewport(256.0, 128.0);
        view.grid_mut().set_total_rows(4);
        view.hgrid_mut().set_columns(40);
        // One short line, so most of the target stays background and the sample is unambiguous.
        let source = Listed(vec!["hi".to_owned()]);

        let (bg_r, bg_g, bg_b) = background_rgb8();
        for frame in 1..=2 {
            // Magenta: a colour no pass here can produce, so "the background was not drawn" is
            // distinguishable from "the background was drawn correctly". See `clear_offscreen`.
            r.gpu.clear_offscreen([1.0, 0.0, 1.0, 1.0]);
            let laid = r.paint_rows(&view, &source).expect("a text frame");
            assert!(laid.quads > 0, "frame {frame} laid out no text at all");
            let pixels = r.gpu.read_back().expect("read back");

            // The bottom-right corner: below the single row of text and right of it, so nothing but
            // background can have been drawn there.
            let [b, g, red, _] = pixels.at(250, 120);
            assert_eq!(
                (red, g, b),
                (bg_r, bg_g, bg_b),
                "frame {frame}: the background is {red},{g},{b} and should be \
                 {bg_r},{bg_g},{bg_b} — a pass that does not set the state it needs has \
                 inherited one from the text pass"
            );
        }
    }

    /// The atlas fills, and it fills *after* the present.
    ///
    /// Without `flush_misses` on this path the cache would never gain a glyph: every frame would
    /// queue the same misses, draw the same placeholder boxes and report the same `queued`. The
    /// second frame's numbers are what distinguish "rasterisation happened" from "the frame merely
    /// returned `Ok`".
    #[cfg(windows)]
    #[test]
    fn the_second_frame_is_warm() {
        let Some((mut r, view)) = renderer_or_skip("the_second_frame_is_warm") else {
            return;
        };
        let line = Listed(vec![
            "the quick brown fox jumps over the lazy dog".to_owned();
            8
        ]);

        let cold = r.paint_rows(&view, &line).expect("cold frame");
        assert!(cold.queued > 0, "a cold atlas queued nothing");
        // **`rasterised` counts glyphs that gained ink, not glyphs that were resolved**, and the
        // difference is the space: `flush_misses` records a blank glyph so it is never asked for
        // again but does not count it as landed. This fixture has one space, so the two numbers
        // differ by exactly one — and asserting equality here was wrong about the code rather than
        // the code being wrong. Resolution is what the next frame proves.
        assert!(
            cold.rasterised > 0 && cold.rasterised <= cold.queued,
            "{} rasterised against {} queued",
            cold.rasterised,
            cold.queued
        );

        let warm = r.paint_rows(&view, &line).expect("warm frame");
        assert_eq!(
            warm.queued, 0,
            "the second frame queued {} glyphs, so the first frame's rasterisation was lost",
            warm.queued
        );
        assert!(warm.quads > 0);
    }

    /// The two stages of the first paint must be the same colour (`SPEC.md` §3.2). They are
    /// necessarily expressed twice — GDI wants 8-bit channels, the render target wants floats —
    /// and if they ever drift apart the handover from the class brush to the renderer becomes a
    /// visible flash on every cold start. That is the whole reason both live in this crate.
    #[test]
    fn both_paint_stages_are_the_same_colour() {
        let (r, g, b) = background_rgb8();
        for (channel, expected) in BACKGROUND[..3].iter().zip([r, g, b]) {
            let as_8bit = (channel * 255.0).round() as u8;
            assert_eq!(
                as_8bit,
                expected,
                "BACKGROUND {BACKGROUND:?} and background_rgb8() {:?} disagree",
                (r, g, b)
            );
        }
    }

    #[test]
    fn background_is_opaque() {
        assert_eq!(
            BACKGROUND[3], 1.0,
            "a translucent background would composite against whatever is behind the window"
        );
    }
}
