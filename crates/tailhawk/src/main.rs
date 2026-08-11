//! Tailhawk — Windows shell. Owns the window, the message loop and input; hands the core a
//! drawable and nothing else (`SPEC.md` §3.1).
//!
//! M0 is the skeleton: a window that opens, a D3D11 device with the WARP fallback, and the
//! two-stage first paint. M1 added reading and decoding. **M3 joins them**: a [`Document`] opens and
//! indexes a log on a worker, and `WM_PAINT` lays a viewport of it out and draws it.
//!
//! Input lives here and only here, per §3.1 — a message becomes a [`Navigate`], and `grid.rs`
//! decides what moving means. Nothing in this file computes a scroll position; `SPEC.md` §6.4 spent
//! two experiments arguing how that arithmetic must be done and it is done there.

// The shipped binary is a GUI app with no console. A test harness is not: as a windows-subsystem
// executable it would have nowhere to print, so the attribute is dropped for `cargo test`.
#![cfg_attr(not(test), windows_subsystem = "windows")]
#![deny(unsafe_op_in_unsafe_fn)]

use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use tailhawk_core::indexer::{build_index, IndexOptions};
use tailhawk_core::{
    background_rgb8, Charset, FileSource, LineIndex, LogFile, Renderer, Rows, View, WindowHandle,
    RENDER_CAP_CELLS,
};
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, InvalidateRect};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::MK_SHIFT;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_B, VK_CONTROL, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT,
    VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, KillTimer,
    LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer, SetWindowPos, SetWindowTextW,
    ShowWindow, SystemParametersInfoW, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    IDC_ARROW, MSG, SPI_GETWHEELSCROLLLINES, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WHEEL_DELTA, WINDOW_EX_STYLE, WM_DESTROY, WM_DPICHANGED,
    WM_KEYDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSW,
    WS_OVERLAPPEDWINDOW,
};

/// Polls for the worker's device. `SPEC.md` §3.2 wants the device off the window thread, so the
/// result has to be collected without blocking the loop — a short timer is the least machinery
/// that does it, and it stops as soon as the device lands.
const DEVICE_POLL_TIMER: usize = 1;
const DEVICE_POLL_MS: u32 = 4;

thread_local! {
    static STATE: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

/// An open log, indexed, with the viewport onto it.
///
/// **The index is built on the worker, not the window thread**, for the same reason the device is:
/// a multi-GB file would otherwise undo the two-stage paint `experiments/g3-d3d11` measured at
/// 13.1 ms. Everything here is `Send`, so it crosses the channel whole once it is ready.
struct Document {
    file: LogFile,
    index: LineIndex,
    charset: Charset,
    rows: Rows,
    view: View,
    summary: String,
}

impl Document {
    /// Opens, detects and indexes. Runs on a worker.
    fn open(path: &std::path::Path) -> std::result::Result<Self, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let describe = |e: &dyn std::fmt::Display| format!("{name}: {e}");

        // Opened once and handed on: two handles could land on different files if the log rotates
        // between them, and the index would then describe bytes the reads no longer return.
        let source = FileSource::open(path).map_err(|e| describe(&e))?;
        let detection = *source.detection();
        let file = source.into_file();
        let charset = detection.charset;
        // **The BOM is consumed, not indexed as content.** Indexing from zero would put its bytes
        // at the start of line 0, which §5.6 says are never rendered — while still leaving every
        // later byte offset exact, which is why the index starts after it rather than shifting.
        let start = detection.bom_len as u64;
        let end = file.len().map_err(|e| describe(&e))?;

        let index = build_index(&file, charset, start, end, &IndexOptions::default())
            .map_err(|e| describe(&e))?;

        let flag = if detection.disagreed { " (mixed?)" } else { "" };
        let summary = format!(
            "{name}: {}{flag}, {} lines, {end} bytes",
            charset.name(),
            index.line_count()
        );

        Ok(Self {
            rows: Rows::new(charset),
            // Metrics arrive with the device; a zero-size view is replaced before the first frame
            // draws, and `View::set_metrics` is what §3.1 requires be driven from the measured face.
            view: View::new(1.0, 1.0),
            index,
            charset,
            file,
            summary,
        })
    }

    /// Points the view at the window and the file, and reads the rows it now shows.
    ///
    /// **The extent is a bound, not a measurement, and it is loose on purpose.** `exact_cells` is
    /// only answerable for an all-ASCII byte-oriented file; anything else falls back to the byte
    /// length, which over-states the column count for UTF-16 and for multi-byte UTF-8. §10.3's
    /// render cap is what keeps that finite, and `hgrid` refines the extent as rows are laid out.
    fn lay_out(&mut self, cell: (f32, f32), size: (u32, u32)) {
        let (cell_w, row_h) = cell;
        self.view.set_metrics(cell_w, row_h);
        self.view.set_viewport(size.0 as f32, size.1 as f32);
        self.view.grid_mut().set_total_rows(self.index.line_count());

        let extent = self.index.extent();
        let columns = extent
            .exact_cells(self.charset)
            .unwrap_or_else(|| extent.max_line_bytes())
            .min(RENDER_CAP_CELLS as u64);
        self.view.hgrid_mut().set_columns(columns);

        let visible: Vec<u64> = self.view.grid().visible().map(|p| p.row).collect();
        let (first, count) = match (visible.first(), visible.len()) {
            (Some(first), n) => (*first, n),
            _ => return,
        };
        // A read that fails does not fail the frame — §11.3. `Rows` keeps what it got and records
        // why the rest is missing; those rows simply draw nothing.
        let anchored = self.view.hgrid().visible_columns().start > 0;
        let _ = self
            .rows
            .fetch(&self.file, &self.index, first, count, anchored);
    }

    /// Applies one navigation intent. Returns whether anything actually moved.
    ///
    /// **The return value is what stops the window repainting on every key.** A `PageDown` at the
    /// end of the file is a no-op, and invalidating for it would burn a frame to draw the same
    /// pixels — which matters here because a frame is a full re-fetch and re-shape of the viewport.
    fn navigate(&mut self, n: Navigate) -> bool {
        let before = (self.view.grid().scroll(), self.view.hgrid().offset_px());
        let row_h = self.view.grid().row_height().max(1.0);
        let page_rows = ((self.view.grid().viewport_px() / row_h).floor() as i64 - 1).max(1);

        match n {
            Navigate::ByPixels(px) => self.view.grid_mut().scroll_by_px(px),
            Navigate::ByRows(d) => self.view.grid_mut().scroll_by_rows(d),
            Navigate::ByPages(d) => self.view.grid_mut().scroll_by_rows(d * page_rows),
            Navigate::ByColumns(d) => self.view.hgrid_mut().scroll_by_columns(d),
            Navigate::DocStart => self.view.grid_mut().scroll_to_row(0),
            Navigate::DocEnd => self.view.grid_mut().scroll_to_bottom(),
            Navigate::LineStart => self.view.hgrid_mut().scroll_to_start(),
            Navigate::LineEnd => self.view.hgrid_mut().scroll_to_end(),
        }

        (self.view.grid().scroll(), self.view.hgrid().offset_px()) != before
    }
}

/// One navigation intent, as the shell reads it from a message.
///
/// **The shell decides what a key means; the core decides what moving means.** `SPEC.md` §3.1 puts
/// the grid in the core, and §6.4's three scroll rules are already implemented and tested there — so
/// nothing here computes a position. This type is the whole of the seam: a `WM_KEYDOWN` becomes a
/// `Navigate`, and `grid.rs` does the arithmetic that §6.4 argues about.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Navigate {
    /// Vertical, in pixels. **Pixels rather than rows even for the wheel**, because §6.4's rule 1
    /// applies deltas to the sub-row remainder and carries into the row index — which is what lets a
    /// precision touchpad's sub-notch delta move the view at all instead of rounding to zero.
    ByPixels(f32),
    ByRows(i64),
    ByColumns(i64),
    /// Screenfuls, less one row of overlap — the line you were reading stays on screen.
    ByPages(i64),
    DocStart,
    /// Also re-enables follow: `Grid::is_following` is *derived* from being at the bottom, so
    /// arriving there is the same event as turning it back on.
    DocEnd,
    LineStart,
    LineEnd,
}

struct Shell {
    /// `None` until the worker hands the device over. While it is `None` the class background
    /// brush is doing the painting — stage one of the two-stage paint.
    renderer: Option<Renderer>,
    pending: Option<Receiver<std::result::Result<Renderer, tailhawk_core::Error>>>,
    /// What the two workers have reported so far. Either can land first, so the title is rebuilt
    /// from both rather than written by whichever finishes.
    driver: Option<String>,
    reading: Option<Receiver<std::result::Result<Document, String>>>,
    file: Option<String>,
    document: Option<Document>,
    /// Set by [`Shell::paint`] when the frame rasterised glyphs, and acted on by `WM_PAINT` **after**
    /// it has validated the update region. See the comment in `paint`.
    needs_frame: bool,
}

impl Shell {
    /// Stage two: adopt the device the moment it arrives, then ask for a repaint.
    fn poll_device(&mut self, hwnd: HWND) {
        let Some(rx) = self.pending.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(mut renderer)) => {
                self.driver = Some(renderer.driver().name().to_owned());
                // **The window's monitor, not the system's.** The renderer is built on a worker
                // before any window exists, so it starts at 100%; a window opened on a 150% monitor
                // never sees a `WM_DPICHANGED` for it, because nothing changed. Reading the DPI on
                // adoption is the only thing that makes the *first* frame correct there.
                renderer.set_dpi(unsafe { GetDpiForWindow(hwnd) });
                self.renderer = Some(renderer);
                self.pending = None;
                self.refresh_title(hwnd);
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            // Device creation failed on every rung of the chain. The window stays up painting
            // stage one rather than dying: `SPEC.md` §3.2 forbids panicking on device trouble.
            Ok(Err(e)) => {
                self.pending = None;
                self.driver = Some(format!("no graphics device ({e})"));
                self.refresh_title(hwnd);
            }
            Err(TryRecvError::Disconnected) => {
                self.pending = None;
                self.driver = Some("graphics worker died".to_owned());
                self.refresh_title(hwnd);
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn poll_file(&mut self, hwnd: HWND) {
        let Some(rx) = self.reading.as_ref() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(document)) => {
                self.reading = None;
                self.file = Some(document.summary.clone());
                self.document = Some(document);
                self.refresh_title(hwnd);
                // The file only becomes visible on the next frame, and nothing else will ask for
                // one — the window is otherwise idle once the device has landed.
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            Ok(Err(e)) => {
                self.reading = None;
                self.file = Some(e);
                self.refresh_title(hwnd);
            }
            Err(TryRecvError::Disconnected) => {
                self.reading = None;
                self.file = Some("read failed".to_owned());
                self.refresh_title(hwnd);
            }
            Err(TryRecvError::Empty) => {}
        }
    }

    fn refresh_title(&self, hwnd: HWND) {
        let mut title = String::from("Tailhawk");
        for part in [self.driver.as_deref(), self.file.as_deref()]
            .into_iter()
            .flatten()
        {
            title.push_str(" — ");
            title.push_str(part);
        }
        set_title(hwnd, &title);
        if self.pending.is_none() && self.reading.is_none() {
            stop_polling(hwnd);
        }
    }

    fn client_size(hwnd: HWND) -> (u32, u32) {
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(hwnd, &mut rc);
        }
        (
            (rc.right - rc.left).max(1) as u32,
            (rc.bottom - rc.top).max(1) as u32,
        )
    }

    /// Returns false when there is no device yet, so the caller can fall through to
    /// `DefWindowProcW` and let the class brush paint stage one.
    fn paint(&mut self, hwnd: HWND) -> bool {
        let Some(renderer) = self.renderer.as_mut() else {
            return false;
        };
        let (w, h) = Self::client_size(hwnd);
        let mut rasterised = 0;
        let drawn = renderer
            .attach(WindowHandle(hwnd.0 as isize), w, h)
            .and_then(|()| match self.document.as_mut() {
                // **The metrics come from the measured face every frame, not once.** §3.1 requires
                // integer cell advances re-derived at the current scale, and a DPI change between
                // frames is exactly the case a cached cell would get wrong.
                Some(doc) => {
                    let cell = renderer.cell()?;
                    doc.lay_out(cell, (w, h));
                    // `Rows` is the row source, so the painter reads its text and its column
                    // anchors by reference — the closure this replaced allocated a `String` per row
                    // per frame and had nowhere to put the anchors at all.
                    let laid = renderer.paint_rows(&doc.view, &doc.rows)?;
                    rasterised = laid.rasterised;
                    Ok(())
                }
                // No file yet: the background, which is all M1 ever drew.
                None => renderer.paint(),
            });
        // **Rasterising is a reason to draw again, and the request cannot be made from in here.**
        // §3.2 puts glyph rasterisation *after* the present, so the first frame on a cold atlas
        // draws a placeholder box in every cell — which is exactly what a screenshot of the first
        // wiring showed: a perfect grid of boxes, right geometry, no text. Nothing else was going
        // to ask for another frame, because an idle window gets one `WM_PAINT` and then silence.
        //
        // Calling `InvalidateRect` here does nothing at all: `WM_PAINT` clears the update region
        // with `ValidateRect` **after** this returns, which wipes it. So the flag is raised and the
        // handler invalidates once it has validated. It converges rather than spinning — the next
        // frame finds those glyphs resident, rasterises nothing and asks for nothing.
        self.needs_frame = rasterised > 0;
        if drawn.is_err() {
            // The renderer rebuilds a lost device itself, so an error here means it tried and
            // gave up. Drop back to stage one rather than tearing the process down — `SPEC.md`
            // §3.2 forbids dying on device trouble, and the class brush still paints.
            self.renderer = None;
            return false;
        }
        // Recovery can move the device onto WARP, and the title is the only place this build
        // says which rung it is on. It is read back rather than remembered for that reason.
        let driver = renderer.driver().name();
        if self.driver.as_deref() != Some(driver) {
            self.driver = Some(driver.to_owned());
            self.refresh_title(hwnd);
        }
        true
    }

    /// Takes a new monitor DPI and repaints if the scale actually changed.
    ///
    /// **The rebuild is not done here.** `ensure_painter` sees the new `px_per_em` on the next
    /// frame and replaces the atlas then, which keeps the scale-change and device-loss paths as one
    /// mechanism rather than two. All this does is record the scale and ask for that frame.
    fn set_dpi(&mut self, hwnd: HWND, dpi: u32) {
        let changed = self
            .renderer
            .as_mut()
            .is_some_and(|renderer| renderer.set_dpi(dpi));
        if changed {
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }

    /// Applies a navigation intent and asks for a frame only if the view moved.
    fn navigate(&mut self, hwnd: HWND, n: Navigate) {
        let moved = self.document.as_mut().is_some_and(|doc| doc.navigate(n));
        if moved {
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }

    fn resize(&mut self, hwnd: HWND) {
        if let Some(renderer) = self.renderer.as_mut() {
            let (w, h) = Self::client_size(hwnd);
            let _ = renderer.resize(w, h);
        }
    }
}

/// The user's lines-per-notch setting.
///
/// **Read every time rather than cached**, because it is a control-panel setting that can change
/// while the app runs, and because the "wheel scrolls a whole screen" accessibility value
/// (`WHEEL_PAGESCROLL`) arrives through the same channel. Hard-coding 3 is what makes an app scroll
/// at a different speed from every other window on the desktop.
fn wheel_lines() -> u32 {
    let mut lines: u32 = 3;
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWHEELSCROLLLINES,
            0,
            Some(std::ptr::addr_of_mut!(lines).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() || lines == 0 {
        return 3;
    }
    // `WHEEL_PAGESCROLL` means "a screen at a time". A page is handled by `ByPages`, and clamping
    // here keeps one notch from flinging the view an arbitrary distance.
    lines.min(64)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn set_title(hwnd: HWND, title: &str) {
    let t = wide(title);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(t.as_ptr()));
    }
}

fn stop_polling(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(hwnd, DEVICE_POLL_TIMER);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER if wparam.0 == DEVICE_POLL_TIMER => {
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.poll_device(hwnd);
                    shell.poll_file(hwnd);
                }
            });
            LRESULT(0)
        }
        WM_PAINT => {
            let (painted, again) = STATE.with(|s| {
                s.borrow_mut()
                    .as_mut()
                    .map(|shell| (shell.paint(hwnd), shell.needs_frame))
                    .unwrap_or((false, false))
            });
            if painted {
                // The swapchain owns the pixels, so there is no BeginPaint/EndPaint pair here;
                // the update region still has to be cleared or the loop spins on WM_PAINT.
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ValidateRect(hwnd, None);
                }
                // **Strictly after the validate.** Invalidating before it is invalidating into a
                // region that is about to be cleared, which is why the first attempt at this
                // changed nothing on screen.
                if again {
                    unsafe {
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
                LRESULT(0)
            } else {
                // Stage one: DefWindowProcW's BeginPaint/EndPaint erases with the class brush,
                // which is the same colour the renderer clears to.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_SIZE => {
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.resize(hwnd);
                }
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
            let delta = (wparam.0 >> 16) as i16 as f32 / WHEEL_DELTA as f32;
            let shift = wparam.0 as u32 & MK_SHIFT.0 != 0;
            let n = if msg == WM_MOUSEHWHEEL {
                // A tilt wheel's positive delta is to the *right*, which is the opposite sign
                // convention from the vertical wheel.
                Navigate::ByColumns((delta * wheel_lines() as f32).round() as i64)
            } else if shift {
                // `UI-DESIGN.md` §12: Shift+wheel is horizontal.
                Navigate::ByColumns(-(delta * wheel_lines() as f32).round() as i64)
            } else {
                // **Pixels, not rows.** §6.4 rule 1 carries the remainder into the row index, so a
                // precision touchpad sending less than a full `WHEEL_DELTA` notch still moves the
                // view instead of rounding to nothing. Positive delta is away from the user, which
                // moves the content *down* and the row index *up*, hence the negation.
                let row_h = STATE.with(|s| {
                    s.borrow()
                        .as_ref()
                        .and_then(|sh| sh.document.as_ref())
                        .map_or(1.0, |d| d.view.grid().row_height())
                });
                Navigate::ByPixels(-delta * wheel_lines() as f32 * row_h)
            };
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.navigate(hwnd, n);
                }
            });
            LRESULT(0)
        }
        WM_KEYDOWN => {
            let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
            // `VIRTUAL_KEY` is a newtype, so its constants cannot appear in a pattern — bare
            // `VK_UP` there binds a variable and matches everything, which compiles into a keyboard
            // where every key is `Up`. Comparing the raw code keeps them as patterns.
            const UP: u16 = VK_UP.0;
            const DOWN: u16 = VK_DOWN.0;
            const LEFT: u16 = VK_LEFT.0;
            const RIGHT: u16 = VK_RIGHT.0;
            const PRIOR: u16 = VK_PRIOR.0;
            const NEXT: u16 = VK_NEXT.0;
            const SPACE: u16 = VK_SPACE.0;
            const B: u16 = VK_B.0;
            const HOME: u16 = VK_HOME.0;
            const END: u16 = VK_END.0;
            // `UI-DESIGN.md` §12's navigation map. Everything else in that table needs a feature
            // that does not exist yet, so it is not bound to a no-op here.
            let n = match wparam.0 as u16 {
                UP => Some(Navigate::ByRows(-1)),
                DOWN => Some(Navigate::ByRows(1)),
                LEFT => Some(Navigate::ByColumns(-1)),
                RIGHT => Some(Navigate::ByColumns(1)),
                PRIOR => Some(Navigate::ByPages(-1)),
                NEXT | SPACE => Some(Navigate::ByPages(1)),
                // `b` for page-up is `less` muscle memory, and the table asks for it by name.
                B => Some(Navigate::ByPages(-1)),
                // Ctrl makes Home/End document extremes; bare, they are line extremes.
                HOME if ctrl => Some(Navigate::DocStart),
                END if ctrl => Some(Navigate::DocEnd),
                HOME => Some(Navigate::LineStart),
                END => Some(Navigate::LineEnd),
                _ => None,
            };
            match n {
                Some(n) => {
                    STATE.with(|s| {
                        if let Some(shell) = s.borrow_mut().as_mut() {
                            shell.navigate(hwnd, n);
                        }
                    });
                    LRESULT(0)
                }
                None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
            }
        }
        WM_DPICHANGED => {
            // **Both halves are required and they are separate.** `lParam` carries the window rect
            // Windows wants for the new scale — honouring it is what makes a drag between monitors
            // land at the right physical size instead of jumping — and the scale itself has to reach
            // the renderer so the atlas is rebuilt and the cell re-measured (§3.1).
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let suggested = lparam.0 as *const RECT;
            if !suggested.is_null() {
                let r = unsafe { *suggested };
                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.set_dpi(hwnd, dpi);
                }
            });
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> Result<()> {
    // **Before anything else, and certainly before any window exists.** `SPEC.md` §3.1 requires
    // per-monitor-V2; without it Windows bitmap-stretches the client area on a non-96-DPI monitor,
    // which for a text viewer means every glyph is resampled and blurry — the one thing this whole
    // atlas exists to avoid. Awareness cannot be changed once a window has been created, so this is
    // the first statement in the process.
    //
    // §3.1 says "declared in the manifest" and this is the API instead: equivalent here because
    // nothing in this process makes a window earlier, and an embedded manifest would mean adding
    // resource compilation to the build for no behavioural gain. Recorded as a deviation in
    // `CLEANROOM.md` rather than passed off as compliance. The result is deliberately ignored — it
    // fails only if awareness is already set, which is not a reason to refuse to start.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    // Device creation starts before the window exists. `experiments/g3-d3d11` measured this
    // ordering as roughly halving time-to-first-pixel, and windows-rs marks the D3D11 interfaces
    // Send, so the renderer crosses the channel directly.
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(Renderer::new());
    });

    // M1's demo, and the dogfood path. A bare positional path only — the option surface is §12.2
    // and lands at M8; guessing at it now would be work thrown away.
    //
    // It reads on a worker for the same reason the device does. A multi-GB file read on the window
    // thread would undo the two-stage paint that `experiments/g3-d3d11` measured at 13.1 ms, and
    // the first log opened this way is meant to be a real one.
    let reading = std::env::args_os().nth(1).map(|arg| {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Document::open(std::path::Path::new(&arg)));
        });
        rx
    });

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)?.into() };
    let class_name = windows::core::w!("TailhawkMain");
    let (r, g, b) = background_rgb8();

    let wc = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        hInstance: instance,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        // Stage one of the two-stage paint: the system erases with this during ShowWindow, before
        // any handler of ours runs and long before a device exists. It must be the same colour the
        // renderer clears to, which is why it comes from the core.
        hbrBackground: unsafe {
            CreateSolidBrush(windows::Win32::Foundation::COLORREF(
                r as u32 | (g as u32) << 8 | (b as u32) << 16,
            ))
        },
        lpszClassName: class_name,
        ..Default::default()
    };
    if unsafe { RegisterClassW(&wc) } == 0 {
        return Err(windows::core::Error::from_win32());
    }

    STATE.with(|s| {
        *s.borrow_mut() = Some(Shell {
            renderer: None,
            pending: Some(rx),
            driver: None,
            reading,

            document: None,

            needs_frame: false,
            file: None,
        });
    });

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("Tailhawk"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1280,
            800,
            None,
            None,
            instance,
            None,
        )?
    };
    unsafe {
        SetTimer(hwnd, DEVICE_POLL_TIMER, DEVICE_POLL_MS, None);
        let _ = ShowWindow(hwnd, SW_SHOW);
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyWindow, WS_OVERLAPPED};

    /// Creates a real, unshown window to hang a swapchain on. Unshown is deliberate: the test
    /// needs a valid `HWND` and a presenting swapchain, not a flash of a window on the desktop of
    /// whoever is running the suite.
    extern "system" fn test_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn hidden_window() -> Option<HWND> {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None).ok()?.into() };
        let class_name = windows::core::w!("TailhawkDeviceLossTest");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(test_wndproc),
            hInstance: instance,
            lpszClassName: class_name,
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            return None;
        }
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name,
                windows::core::w!("tailhawk device loss test"),
                WS_OVERLAPPED,
                0,
                0,
                320,
                240,
                None,
                None,
                instance,
                None,
            )
        }
        .ok()
    }

    /// The half of `SPEC.md` §3.2's device-removed recovery that the core cannot test on its own.
    ///
    /// The core's own tests rebuild a device with no window attached, which leaves the riskiest
    /// part uncovered: after a device is removed, the DXGI factory that made the swapchain is
    /// stale, and a renderer that reuses it comes back "recovered" while presenting to nothing.
    /// Only a crate that may own an `HWND` can catch that, and by §3.1 that is the shell.
    ///
    /// It skips loudly rather than failing where there is no device or no window station — a
    /// headless CI runner is a real possibility and a silently-green device test is worse than an
    /// absent one.
    #[test]
    fn a_device_lost_with_a_window_attached_comes_back_presenting() {
        let mut renderer = match Renderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED a_device_lost_with_a_window_attached_comes_back_presenting: no D3D11 device ({e})");
                return;
            }
        };
        let Some(hwnd) = hidden_window() else {
            eprintln!("SKIPPED a_device_lost_with_a_window_attached_comes_back_presenting: no window station");
            return;
        };

        let window = WindowHandle(hwnd.0 as isize);
        renderer.attach(window, 320, 240).expect("attach");
        renderer.paint().expect("the first frame presents");
        assert_eq!(renderer.device_generation(), 1);

        renderer.simulate_device_loss();
        renderer
            .paint()
            .expect("a lost device is rebuilt and the frame is redrawn, not reported");
        assert_eq!(
            renderer.device_generation(),
            2,
            "the device should have been replaced"
        );

        // A swapchain rebuilt from a stale factory can still present the frame it was made for.
        // Resizing and presenting again is what a swapchain belonging to a dead device cannot do.
        renderer.resize(400, 300).expect("resize after recovery");
        renderer.paint().expect("a frame after recovery and resize");
        assert_eq!(
            renderer.device_generation(),
            2,
            "nothing after the rebuild should have needed another one"
        );

        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    /// A real file on disk, because `Document` owns a `LogFile` and there is no seam for a fake one
    /// — the index and the reads have to agree about the same bytes, which is the point.
    fn scratch_log(name: &str, lines: usize) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(name);
        let mut text = String::new();
        for i in 0..lines {
            text.push_str(&format!("line {i} — a log record with some width to it\n"));
        }
        std::fs::write(&path, text).expect("write the fixture");
        path
    }

    /// The navigation arithmetic, without a window or a device.
    ///
    /// `Document::navigate` is where a key becomes a movement, and it is the half of input handling
    /// that can be tested — the `WM_KEYDOWN` → [`Navigate`] mapping needs a message pump and is
    /// covered only by running the app. The cell metrics are supplied rather than measured, so this
    /// does not need a face either.
    #[test]
    fn navigating_moves_by_the_amounts_the_key_map_promises() {
        let path = scratch_log("tailhawk_nav_test.log", 5_000);
        let mut doc = Document::open(&path).expect("open the fixture");
        // 20 rows of 10 px in a 200 px viewport.
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.index.line_count(), 5_000);
        assert_eq!(doc.view.grid().scroll().row, 0);

        // **A page is a screenful less one row**, so the line you were reading stays on screen.
        assert!(doc.navigate(Navigate::ByPages(1)));
        assert_eq!(doc.view.grid().scroll().row, 19);

        assert!(doc.navigate(Navigate::ByRows(1)));
        assert_eq!(doc.view.grid().scroll().row, 20);

        assert!(doc.navigate(Navigate::ByPages(-1)));
        assert_eq!(doc.view.grid().scroll().row, 1);

        // At the top, going up again moves nothing — and says so, which is what stops the window
        // repainting on a held arrow key.
        assert!(doc.navigate(Navigate::ByRows(-1)));
        assert_eq!(doc.view.grid().scroll().row, 0);
        assert!(
            !doc.navigate(Navigate::ByRows(-1)),
            "a no-op move reported movement, so every key would burn a frame"
        );
        assert!(!doc.navigate(Navigate::DocStart));

        // The end clamps to the last screenful rather than scrolling into blank space, and arriving
        // there *is* following — `Grid::is_following` is derived from being at the bottom.
        assert!(doc.navigate(Navigate::DocEnd));
        assert!(doc.view.grid().is_following());
        assert!(!doc.navigate(Navigate::ByPages(1)), "scrolled past the end");
        let last = doc.view.grid().visible().last().expect("a visible row").row;
        assert_eq!(last, 4_999, "the last row is not the last line");

        // Scrolling up from the bottom drops follow, which `UI-DESIGN.md` §12 requires.
        assert!(doc.navigate(Navigate::ByRows(-1)));
        assert!(!doc.view.grid().is_following());

        let _ = std::fs::remove_file(&path);
    }

    /// The horizontal axis, which has its own extremes and its own key bindings.
    #[test]
    fn the_horizontal_extremes_are_the_line_extremes() {
        let path = scratch_log("tailhawk_nav_h_test.log", 200);
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (200, 200));

        assert!(!doc.navigate(Navigate::LineStart), "already at the start");
        assert!(doc.navigate(Navigate::ByColumns(3)));
        assert!(doc.navigate(Navigate::LineStart));
        assert_eq!(doc.view.hgrid().offset_px(), 0.0);

        assert!(doc.navigate(Navigate::LineEnd));
        assert!(doc.view.hgrid().offset_px() > 0.0, "End went nowhere");
        assert!(
            !doc.navigate(Navigate::ByColumns(1)),
            "scrolled past the end"
        );

        let _ = std::fs::remove_file(&path);
    }
}
