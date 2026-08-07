//! Tailhawk — Windows shell. Owns the window, the message loop and input; hands the core a
//! drawable and nothing else (`SPEC.md` §3.1).
//!
//! M0 is the skeleton: a window that opens, a D3D11 device with the WARP fallback, and the
//! two-stage first paint. M1 adds reading and decoding, which is headless — the only thing the
//! shell does with it is report what the core found, in the title bar, because there is no grid to
//! render it into until M3.

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
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, KillTimer,
    LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer, SetWindowTextW, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, SW_SHOW,
    WINDOW_EX_STYLE, WM_DESTROY, WM_PAINT, WM_SIZE, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
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
        let _ = self.rows.fetch(&self.file, &self.index, first, count);
    }
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
            Ok(Ok(renderer)) => {
                self.driver = Some(renderer.driver().name().to_owned());
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
                    let rows = &doc.rows;
                    let laid =
                        renderer.paint_rows(&doc.view, |row| rows.line(row).map(str::to_owned))?;
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

    fn resize(&mut self, hwnd: HWND) {
        if let Some(renderer) = self.renderer.as_mut() {
            let (w, h) = Self::client_size(hwnd);
            let _ = renderer.resize(w, h);
        }
    }
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
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn main() -> Result<()> {
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
}
