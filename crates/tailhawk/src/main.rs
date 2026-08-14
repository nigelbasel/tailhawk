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

use tailhawk_core::set::LogSet;
use tailhawk_core::stdin::{reap_orphans, stdin as stdin_kind, Pump, StreamEnd};
use tailhawk_core::{
    background_rgb8, Renderer, RowEnd, RowSource, Selection, View, WindowHandle, RENDER_CAP_CELLS,
};
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, InvalidateRect};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MK_SHIFT};
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, VK_B, VK_C, VK_CONTROL, VK_DOWN,
    VK_END, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_SPACE, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW, GetScrollInfo,
    KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer, SetWindowPos,
    SetWindowTextW, ShowWindow, SystemParametersInfoW, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, IDC_ARROW, MSG, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP,
    SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_DISABLENOSCROLL, SIF_PAGE,
    SIF_POS, SIF_RANGE, SIF_TRACKPOS, SPI_GETWHEELSCROLLLINES, SWP_NOACTIVATE, SWP_NOZORDER,
    SW_SHOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, WHEEL_DELTA, WINDOW_EX_STYLE, WM_DESTROY,
    WM_DPICHANGED, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSW,
    WS_OVERLAPPEDWINDOW, WS_VSCROLL,
};

/// Polls for the worker's device. `SPEC.md` §3.2 wants the device off the window thread, so the
/// result has to be collected without blocking the loop — a short timer is the least machinery
/// that does it, and it stops as soon as the device lands.
const DEVICE_POLL_TIMER: usize = 1;
const DEVICE_POLL_MS: u32 = 4;

/// Polls the file for new bytes. 100 ms is well under what a reader notices and well over what a
/// `GetFileSizeEx` on an open handle costs, and it keeps tailing off the critical path entirely.
const FOLLOW_TIMER: usize = 2;
const FOLLOW_POLL_MS: u32 = 100;

/// Scrollbar units. Fixed rather than the row count, so no file size can overflow `SCROLLINFO`'s
/// `i32` -- the grid speaks in fractions at both ends and Win32 never sees a row number.
const SCROLL_RANGE: i32 = 10_000;

thread_local! {
    static STATE: RefCell<Option<Shell>> = const { RefCell::new(None) };
}

/// An open log, indexed, with the viewport onto it.
///
/// **The index is built on the worker, not the window thread**, for the same reason the device is:
/// a multi-GB file would otherwise undo the two-stage paint `experiments/g3-d3d11` measured at
/// 13.1 ms. Everything here is `Send`, so it crosses the channel whole once it is ready.
struct Document {
    /// The source, which is **a set of files and not one file** — §5.5b. A log with nothing beside
    /// it is a set of one, so there is a single path here rather than two.
    set: LogSet,
    view: View,
    summary: String,
    /// The current selection, or `None` for "nothing selected".
    ///
    /// A caret — an empty selection at a click — is `Some`, not `None`: `Selection::at` exists so a
    /// click that selects nothing still records **where** the next shift-click extends from.
    selection: Option<Selection>,
    /// Whether the left button is down, so `WM_MOUSEMOVE` can tell a drag from a hover.
    dragging: bool,
    /// The stdin pump, when the source is a pipe — §4.2. Held for its whole life because dropping
    /// it deletes the spill (§13.2), and the `LogSet` above is reading that file.
    pump: Option<Pump>,
    /// Whether the producer had closed its end as of the last tick.
    ///
    /// **A remembered edge, not a query.** The title only redraws when a tick reports a change, and
    /// end-of-stream changes nothing about the file — so without this the window that has stopped
    /// growing goes on saying "reading stdin", which is what a hung window looks like.
    stream_done: bool,
}

impl RowSource for Document {
    fn row_text(&self, row: u64) -> Option<&str> {
        self.set.row_text(row)
    }

    fn row_anchors(&self, row: u64) -> &tailhawk_core::cell::ColumnAnchors {
        self.set.row_anchors(row)
    }

    /// **The one thing `Rows` cannot answer**, and the reason `Document` is now the painter's source
    /// rather than `Rows` directly: the selection lives here, the text lives there, and the painter
    /// needs both for the same row in the same frame.
    ///
    /// `usize::MAX` carries [`RowEnd::ToLineEnd`], so the painter can tint to the right edge without
    /// being told how long the line is.
    fn row_selection(&self, row: u64) -> Option<std::ops::Range<usize>> {
        let span = self.selection?.row_span(row)?;
        let end = match span.end {
            RowEnd::ToLineEnd => usize::MAX,
            RowEnd::Cell(cell) => cell,
        };
        (span.start_cell < end).then_some(span.start_cell..end)
    }
}

impl Document {
    /// Opens, detects and indexes. Runs on a worker.
    fn open(path: &std::path::Path) -> std::result::Result<Self, String> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        // **A set, not a file** — §5.5b. `LogSet` infers the members from the siblings, opens each
        // with its own encoding detection and index, and gives them one row space; a log with
        // nothing beside it comes back as a set of one, so there is no second path here.
        let set = LogSet::open(path).map_err(|e| format!("{name}: {e}"))?;

        // **Only the name is fixed.** Everything else in the title -- the encoding, the membership,
        // the counts -- can change while the log is being followed, so `describe` formats them per
        // read. A tail whose title says "1 lines" while sixty scroll past is worse than no title at
        // all, and after §5.5b's roll the same is true of the file list.
        let summary = name;

        Ok(Self {
            // Metrics arrive with the device; a zero-size view is replaced before the first frame
            // draws, and `View::set_metrics` is what §3.1 requires be driven from the measured face.
            view: View::new(1.0, 1.0),
            set,
            summary,
            selection: None,
            dragging: false,
            pump: None,
            stream_done: false,
        })
    }

    /// Opens the standard input stream — `SPEC.md` §4.2.
    ///
    /// The pump spills to a temp file and this opens **that file**, so following a pipe is following
    /// a file and nothing downstream knows the difference. §4.2 asks for exactly that: the spill
    /// "reuses the same index path as a real file".
    ///
    /// **`open_single`, not `open`.** Spill names share a shape, so a rolling-set inference would
    /// adopt a *concurrent* instance's spill as older history — another user's piped stream spliced
    /// into this one's scrollback. `set.rs` argues it at `open_single` and `stdin.rs` tests it.
    ///
    /// The spill is empty at this point and that is fine: an empty file is a source with no rows,
    /// and `Follow::after_build` already seeds line 0 for one — the case that had its own bug and
    /// its own test back when following was written.
    fn from_pipe() -> std::result::Result<Self, String> {
        let pump = Pump::start().map_err(|e| format!("stdin: {e}"))?;
        let set = LogSet::open_single(pump.path()).map_err(|e| format!("stdin: {e}"))?;
        Ok(Self {
            view: View::new(1.0, 1.0),
            set,
            // §13.2: "The spill location is displayed in source properties, because a user piping
            // production logs deserves to know where they landed." There are no source properties
            // yet, so the title carries it — a user piping production logs is told where the bytes
            // are, in the only place there is to tell them.
            summary: format!("<stdin> → {}", pump.path().display()),
            selection: None,
            dragging: false,
            pump: Some(pump),
            stream_done: false,
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
        self.view.grid_mut().set_total_rows(self.set.total_rows());

        // Across the whole set, not just the live member: §5.5b's scrollback reaches into files
        // whose widest line may be wider than anything the current one holds.
        let extent = self.set.extent();
        let columns = extent
            .exact_cells(self.set.charset())
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
        let _ = self.set.fetch(first, count, anchored);
    }

    /// The title text, rebuilt from the live state — every part of it except the name.
    ///
    /// §5.5b requires the inferred set be "shown in the UI for confirmation rather than silently
    /// assumed", and the title bar is the only UI this has. It names the oldest and newest member so
    /// the direction can be checked against the folder, rather than asking the user to take the word
    /// "ascending" on trust.
    ///
    /// **Rebuilt rather than cached because a set that rolls is a different set.** Freezing this at
    /// open put "2 files … newest is `log_002.txt`" in the title of a window showing three, and a
    /// stale confirmation is worse than none — it invites a check against a list that has moved on.
    fn describe(&self) -> String {
        let flag = if self.set.newest().disagreed() {
            " (mixed?)"
        } else {
            ""
        };
        // §4.2: end of stream "is **not** an app exit". Saying so in the title is what stops a
        // window that has stopped growing looking like a window that has hung.
        // A pipe is one file by construction, so §5.5b's set description says nothing a user of it
        // wants — the spill's path is already in `summary`, which is the part §13.2 asks for.
        let source = match self.pump.as_ref().map(|p| (p.finished(), p.outcome())) {
            // **A stream that broke does not look like one that finished**, which is the
            // distinction `PLAN.md` asks a pipe source to make. A pipe cannot tell a producer that
            // exited cleanly from one that was killed — both just close the handle — but it can
            // tell either of those from a read or a spill that failed, and that is what this says.
            Some((_, Some(StreamEnd::Failed(why)))) => format!(" — stream failed: {why}"),
            Some((true, _)) => " — stream complete".to_string(),
            Some((false, _)) => " — reading stdin".to_string(),
            None => format!(" — {}", self.set.describe()),
        };
        format!(
            "{}: {}{flag}{source}, {} lines, {} bytes",
            self.summary,
            self.set.charset().name(),
            self.set.total_rows(),
            self.set.bytes()
        )
    }

    /// Advances the source one tick: growth, rotation, rolls and retention, per §5.5 and §5.5b.
    ///
    /// **`was_following` is read before the row count changes, and that ordering is the whole of
    /// tailing.** `Grid::is_following` is derived from being at the bottom, so once `set_total_rows`
    /// has taken the new count the old position is no longer the bottom and the answer is always
    /// false. Asking first is what distinguishes "the user is watching the tail" from "the user
    /// scrolled up", and getting it the wrong way round would either pin a reader who had scrolled
    /// back or fail to follow at all.
    ///
    /// Time-bounded rather than byte-bounded — see `Follow::poll_for`. 30 ms of a 100 ms tick leaves
    /// the message loop 70% idle and is what carries the 50 MB/s of `SPEC.md` §11.3's criterion; a
    /// single byte-budgeted scan per tick capped throughput at roughly 40 MB/s.
    fn poll_follow(&mut self) -> bool {
        let was_following = self.view.grid().is_following();

        // **Read before the scan, not after.** The pump flushes each read and only then sets its
        // finished flag, so a `true` seen *here* guarantees every byte is already on disk and the
        // scan below will pick it up. Asking afterwards could report "stream complete" in the same
        // tick whose length check ran a moment too early — a title that says the stream is finished
        // while its last line is missing.
        let finished_now = self.pump.as_ref().is_some_and(Pump::finished);
        let stream_changed = finished_now != self.stream_done;
        self.stream_done = finished_now;

        let polled = self.set.poll();
        if polled.is_quiet() {
            // The rows did not move, but the title may still be wrong. §4.2's end of stream "is not
            // an app exit", and a window that stops updating without saying why looks like one that
            // has hung.
            return stream_changed;
        }

        // **A selection addresses rows, and these two events move rows out from under it.** A
        // truncated member's rows are different bytes at the same numbers; a retirement renumbers
        // everything after it. Keeping the selection would tint text the user never chose, and
        // §5.6's "copy preserves the original bytes" would be copying the wrong file's.
        //
        // A *roll* does not clear it: §5.5b appends the new member at the end of the row space, so
        // every existing row keeps its number, which is the property the prefix sum was chosen for.
        if polled.reset || !polled.retired.is_empty() {
            self.selection = None;
            self.dragging = false;
        }

        self.view.grid_mut().set_total_rows(self.set.total_rows());
        if was_following {
            // A tail that rolls keeps tailing. §5.5b wants a separator row at the boundary too,
            // which is a rendering feature and is not done — `LogSet::locate` reports where one
            // goes, and `HANDOFF.md` records that nothing draws it.
            self.view.grid_mut().scroll_to_bottom();
        }
        true
    }

    /// Starts, extends or replaces the selection from a click in the client area.
    ///
    /// `x` and `y` are client-relative device pixels, which is what `View::position_at` takes — the
    /// grid has no other origin, and converting anywhere else is one more place to get the scroll
    /// offset wrong. A click outside the drawn rows, or right of the widest line, is **not** a
    /// position and is ignored rather than clamped: inventing one puts the caret where the user did
    /// not click.
    fn select(&mut self, x: f32, y: f32, what: Selecting) -> bool {
        let Some(at) = self.view.position_at(x, y) else {
            return false;
        };
        let before = self.selection;
        match what {
            Selecting::Start => {
                self.selection = Some(Selection::at(at));
                self.dragging = true;
            }
            Selecting::Extend => {
                if let Some(sel) = self.selection.as_mut() {
                    sel.set_focus(at);
                } else {
                    self.selection = Some(Selection::at(at));
                }
            }
            Selecting::Word => {
                // A word needs the row's text, and only the fetched window has it. Falling back to a
                // caret is honest; guessing a span from a line we cannot see is not.
                self.selection = Some(match self.set.row_text(at.row) {
                    Some(line) => Selection::word(self.view.cells(), at.row, line, at.cell),
                    None => Selection::at(at),
                });
            }
            Selecting::Line => self.selection = Some(Selection::line(at.row)),
        }
        self.selection != before
    }

    /// The selected text, exactly as it is in the file.
    ///
    /// **§5.6: the bytes are the file's, not a re-rendering of them**, which is why this goes
    /// through `Selection::byte_range` rather than slicing by column. `byte_range` rounds both ends
    /// *outwards* to whole clusters, so a zero-width character sitting on the boundary — the bidi
    /// override of §13.4's Trojan Source line — is copied rather than silently dropped. Slicing by
    /// column would drop it, and the user would paste something that reads differently from what
    /// they selected.
    ///
    /// Rows are joined with `\n` rather than the file's own terminators, because a selection can
    /// span rows whose terminators disagree; `RowSpan::line_break` says whether a row's break is
    /// inside the selection at all.
    ///
    /// **⚠ Only rows in the fetched window can be copied.** `Rows` holds one screenful, so a
    /// selection dragged past it yields the rows it can see. A real limit, recorded in `HANDOFF.md`.
    fn copy_text(&self) -> Option<String> {
        let sel = self.selection?;
        if sel.is_empty() {
            return None;
        }
        let mut out = String::new();
        for row in sel.first_row()..=sel.last_row() {
            let Some(line) = self.set.row_text(row) else {
                continue;
            };
            if let Some(bytes) = sel.byte_range(self.view.cells(), row, line) {
                out.push_str(&line[bytes]);
            }
            if sel.row_span(row).is_some_and(|s| s.line_break) {
                out.push('\n');
            }
        }
        (!out.is_empty()).then_some(out)
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

/// What a mouse event means for the selection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Selecting {
    /// Left button down: a caret, and the anchor a drag will extend from.
    Start,
    /// Dragging, or shift-clicking: move the focus and keep the anchor.
    Extend,
    /// Double-click.
    Word,
    /// Triple-click.
    Line,
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
    /// When and where the last double-click landed, so the next click can be read as a triple.
    last_double: Option<(u32, f32, f32)>,
    needs_frame: bool,
    /// How long recent frames took. See [`Frames`].
    frames: Frames,
}

/// A ring of recent frame durations, and the reason there is one.
///
/// **`PLAN.md`'s M4 criterion is "50 MB/s for 60 s *without dropped frames*", and until now that was
/// being scored with an instrument that cannot answer it.** The throughput rig measures a
/// `SendMessageW(WM_NULL)` round-trip from the writer process, which blocks until the window thread
/// is free — so it counts a `Present` that is *deliberately* vsync-blocked exactly the same as a
/// scan that has seized the thread. A healthy 60 fps application shows round-trips up to 16.7 ms as
/// a matter of course, and the rig reported a p95 of 17.3 ms on an idle window at 1 MB/s.
///
/// So this measures the thing the criterion is about: **how long a frame takes inside the window**,
/// which excludes the time the window spends idle waiting for a message and includes everything it
/// actually does. A frame over [`FRAME_BUDGET_MS`] is one the user could have noticed.
///
/// It is 480 frames — eight seconds at 60 fps — because the interesting question is "how is it doing
/// *now*", not over the whole run. A whole-run average hides a stall inside a minute of health.
struct Frames {
    /// Durations in microseconds, newest overwriting oldest.
    ring: [u32; FRAME_SAMPLES],
    at: usize,
    filled: usize,
    /// Frames over budget since the process started. A running total, because "it has stuttered 4
    /// times in ten minutes" is a different fact from "it is stuttering now" and both are wanted.
    over_budget: u64,
}

/// A frame this long or longer is one a 60 Hz display could not show on time.
const FRAME_BUDGET_MS: u32 = 17;

/// Eight seconds at 60 fps.
const FRAME_SAMPLES: usize = 480;

impl Frames {
    fn new() -> Self {
        Self {
            ring: [0; FRAME_SAMPLES],
            at: 0,
            filled: 0,
            over_budget: 0,
        }
    }

    fn record(&mut self, took: std::time::Duration) {
        let micros = took.as_micros().min(u32::MAX as u128) as u32;
        self.ring[self.at] = micros;
        self.at = (self.at + 1) % FRAME_SAMPLES;
        self.filled = (self.filled + 1).min(FRAME_SAMPLES);
        if micros >= FRAME_BUDGET_MS * 1000 {
            self.over_budget += 1;
        }
    }

    /// p95 and worst of the recent window, in milliseconds, plus the lifetime over-budget count.
    fn summary(&self) -> Option<(f32, f32, u64)> {
        if self.filled == 0 {
            return None;
        }
        let mut sample: Vec<u32> = self.ring[..self.filled].to_vec();
        sample.sort_unstable();
        let p95 = sample[(sample.len() * 95 / 100).min(sample.len() - 1)];
        let worst = *sample.last().expect("non-empty");
        Some((p95 as f32 / 1000.0, worst as f32 / 1000.0, self.over_budget))
    }
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
                self.file = Some(document.describe());
                self.document = Some(document);
                self.refresh_title(hwnd);
                // Tailing starts the moment there is something to tail.
                unsafe {
                    SetTimer(hwnd, FOLLOW_TIMER, FOLLOW_POLL_MS, None);
                }
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
        // **The frame instrument, where a user and a measurement rig can both see it.** M4 asks for
        // "without dropped frames" and nothing in the product could say whether that held; the
        // throughput rig could only measure how long the window took to answer a message, which
        // counts a vsync-blocked Present the same as a seized thread.
        if let Some((p95, worst, over)) = self.frames.summary() {
            title.push_str(&format!(
                " — frame p95 {p95:.1} ms, worst {worst:.1} ms, {over} over budget"
            ));
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
        let began = std::time::Instant::now();
        let painted = self.paint_inner(hwnd);
        self.frames.record(began.elapsed());
        painted
    }

    fn paint_inner(&mut self, hwnd: HWND) -> bool {
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
                    let laid = renderer.paint_rows(&doc.view, &*doc)?;
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

    fn is_dragging(&self) -> bool {
        self.document.as_ref().is_some_and(|d| d.dragging)
    }

    fn end_drag(&mut self) {
        if let Some(doc) = self.document.as_mut() {
            doc.dragging = false;
        }
    }

    /// Applies a selection change, and reports whether anything changed so a frame can be skipped.
    fn select(&mut self, x: f32, y: f32, what: Selecting) -> bool {
        self.document
            .as_mut()
            .is_some_and(|doc| doc.select(x, y, what))
    }

    /// Puts the selection on the clipboard as `CF_UNICODETEXT`.
    ///
    /// **The clipboard takes ownership of the handle when `SetClipboardData` succeeds**, so the
    /// `GlobalFree` is on the failure path only — freeing after a successful hand-over is a double
    /// free of memory the system now owns. `CloseClipboard` runs on every path, including the ones
    /// that fail, because leaving it open locks every other application out of the clipboard.
    fn copy(&self) -> bool {
        let Some(text) = self.document.as_ref().and_then(Document::copy_text) else {
            return false;
        };
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = std::mem::size_of_val(wide.as_slice());

        unsafe {
            if OpenClipboard(None).is_err() {
                return false;
            }
            let _ = EmptyClipboard();
            let Ok(handle) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
                let _ = CloseClipboard();
                return false;
            };
            let dst = GlobalLock(handle);
            if dst.is_null() {
                let _ = GlobalFree(handle);
                let _ = CloseClipboard();
                return false;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), dst.cast::<u16>(), wide.len());
            let _ = GlobalUnlock(handle);

            let ok = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(handle.0)).is_ok();
            if !ok {
                let _ = GlobalFree(handle);
            }
            let _ = CloseClipboard();
            ok
        }
    }

    /// Points the scrollbar at where the view actually is.
    ///
    /// **The range is a fixed 0..[`SCROLL_RANGE`] rather than the row count**, and that is the whole
    /// trick: `SCROLLINFO` is `i32`, so a 50M-line file is fine but a large enough one would not be,
    /// and scaling by row count would put the overflow in the future rather than removing it. The
    /// grid already speaks in fractions — `thumb_fraction` and `scroll_to_fraction` — so the
    /// scrollbar is a fraction at both ends and the row count never reaches Win32 at all.
    fn sync_scrollbar(&self, hwnd: HWND) {
        let Some(doc) = self.document.as_ref() else {
            return;
        };
        let grid = doc.view.grid();
        let total = grid.total_rows();
        let page = grid.page_rows().max(1);
        // A file that fits on screen gets a full-width thumb and no travel, which is what
        // `nPage >= nMax` means to Win32 — it disables the bar rather than showing a false position.
        let page_units = if total <= page {
            SCROLL_RANGE
        } else {
            ((SCROLL_RANGE as u64 * page) / total).max(1) as i32
        };
        let info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS | SIF_DISABLENOSCROLL,
            nMin: 0,
            nMax: SCROLL_RANGE,
            nPage: page_units as u32,
            nPos: (grid.thumb_fraction() * SCROLL_RANGE as f32).round() as i32,
            nTrackPos: 0,
        };
        unsafe { SetScrollInfo(hwnd, SB_VERT, &info, true) };
    }

    /// Applies a navigation intent and asks for a frame only if the view moved.
    fn navigate(&mut self, hwnd: HWND, n: Navigate) {
        let moved = self.document.as_mut().is_some_and(|doc| doc.navigate(n));
        if moved {
            self.sync_scrollbar(hwnd);
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

/// Milliseconds since boot, for the triple-click window. The caller uses `wrapping_sub`, which
/// is correct across the 49-day rollover.
fn now_ms() -> u32 {
    unsafe { GetTickCount() }
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
        WM_TIMER if wparam.0 == FOLLOW_TIMER => {
            // **The scan is bounded, so this cannot hold the message loop.** `Follow::poll` stops at
            // its byte budget and says whether more is waiting; a writer producing faster than the
            // tick just takes several ticks to catch up, which is §11.3's requirement — the UI stays
            // responsive, not every append lands in one go.
            let grew = STATE.with(|s| {
                s.borrow_mut()
                    .as_mut()
                    .and_then(|shell| shell.document.as_mut())
                    .is_some_and(Document::poll_follow)
            });
            if grew {
                // The counts moved, so the title is now wrong until it is rebuilt.
                STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    if let Some(shell) = state.as_mut() {
                        if let Some(doc) = shell.document.as_ref() {
                            shell.file = Some(doc.describe());
                        }
                        shell.refresh_title(hwnd);
                        shell.sync_scrollbar(hwnd);
                    }
                });
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let (painted, again) = STATE.with(|s| {
                s.borrow_mut()
                    .as_mut()
                    .map(|shell| (shell.paint(hwnd), shell.needs_frame))
                    .unwrap_or((false, false))
            });
            // **After `paint`, because the layout is what decides the row count and page size.**
            // `Document::lay_out` sets the viewport and total rows from the window and the index,
            // so a thumb synced earlier describes the previous frame — visibly wrong on the first
            // paint after opening a file, and after every resize. It is here rather than inside
            // `paint` because the renderer is mutably borrowed for the whole of that function.
            STATE.with(|s| {
                if let Some(shell) = s.borrow().as_ref() {
                    shell.sync_scrollbar(hwnd);
                }
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
            const C: u16 = VK_C.0;

            // `UI-DESIGN.md` §12: Ctrl+C copies the selection raw. Handled before the navigation
            // map because C is not a navigation key and must not fall through to DefWindowProcW.
            if ctrl && wparam.0 as u16 == C {
                STATE.with(|s| {
                    if let Some(shell) = s.borrow().as_ref() {
                        shell.copy();
                    }
                });
                return LRESULT(0);
            }
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
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_MOUSEMOVE | WM_LBUTTONUP => {
            // Client-relative already, and **signed**: a drag above the window gives a negative y,
            // which `position_at` rejects rather than clamping to row 0.
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            let shift = wparam.0 as u32 & MK_SHIFT.0 != 0;
            let held = wparam.0 as u32 & MK_LBUTTON.0 != 0;

            STATE.with(|s| {
                let mut state = s.borrow_mut();
                let Some(shell) = state.as_mut() else {
                    return;
                };
                let moved = match msg {
                    WM_LBUTTONDOWN => {
                        // **Capture, so a drag that leaves the window still ends.** Without it the
                        // button-up goes to whatever is under the pointer and the grid stays stuck
                        // in dragging for ever.
                        unsafe { SetCapture(hwnd) };
                        // **Windows has no triple-click message**, so the third click arrives as an
                        // ordinary button-down and is recognised here: within the system
                        // double-click time, and close enough that a click elsewhere is not
                        // swallowed. Both come from the system, so it matches the user's settings.
                        let triple = shell.last_double.is_some_and(|(t, dx, dy)| {
                            now_ms().wrapping_sub(t) <= unsafe { GetDoubleClickTime() }
                                && (x - dx).abs() < 4.0
                                && (y - dy).abs() < 4.0
                        });
                        let what = if triple {
                            shell.last_double = None;
                            Selecting::Line
                        } else if shift {
                            Selecting::Extend
                        } else {
                            Selecting::Start
                        };
                        shell.select(x, y, what)
                    }
                    WM_LBUTTONDBLCLK => {
                        shell.last_double = Some((now_ms(), x, y));
                        shell.select(x, y, Selecting::Word)
                    }
                    WM_MOUSEMOVE if held && shell.is_dragging() => {
                        shell.select(x, y, Selecting::Extend)
                    }
                    WM_LBUTTONUP => {
                        unsafe {
                            let _ = ReleaseCapture();
                        }
                        shell.end_drag();
                        false
                    }
                    _ => false,
                };
                if moved {
                    unsafe {
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
            });
            LRESULT(0)
        }
        WM_VSCROLL => {
            let code = (wparam.0 & 0xFFFF) as u32;
            STATE.with(|s| {
                let mut state = s.borrow_mut();
                let Some(shell) = state.as_mut() else {
                    return;
                };
                let Some(doc) = shell.document.as_mut() else {
                    return;
                };
                const LINEUP: i32 = SB_LINEUP.0;
                const LINEDOWN: i32 = SB_LINEDOWN.0;
                const PAGEUP: i32 = SB_PAGEUP.0;
                const PAGEDOWN: i32 = SB_PAGEDOWN.0;
                const TOP: i32 = SB_TOP.0;
                const BOTTOM: i32 = SB_BOTTOM.0;
                const THUMBTRACK: i32 = SB_THUMBTRACK.0;
                const THUMBPOSITION: i32 = SB_THUMBPOSITION.0;
                let moved = match code as i32 {
                    LINEUP => doc.navigate(Navigate::ByRows(-1)),
                    LINEDOWN => doc.navigate(Navigate::ByRows(1)),
                    PAGEUP => doc.navigate(Navigate::ByPages(-1)),
                    PAGEDOWN => doc.navigate(Navigate::ByPages(1)),
                    TOP => doc.navigate(Navigate::DocStart),
                    BOTTOM => doc.navigate(Navigate::DocEnd),
                    // **`SB_THUMBTRACK`, not only `SB_THUMBPOSITION`.** Handling the position alone
                    // makes the view jump when the drag ends rather than following the thumb, which
                    // reads as a broken scrollbar. `nTrackPos` is the live position and is the only
                    // place it can be read from — `wParam`'s high word is 16-bit and would quantise
                    // a 50M-line file into 65,536 steps.
                    THUMBTRACK | THUMBPOSITION => {
                        let mut info = SCROLLINFO {
                            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                            fMask: SIF_TRACKPOS,
                            ..Default::default()
                        };
                        unsafe { GetScrollInfo(hwnd, SB_VERT, &mut info) }
                            .is_ok()
                            .then(|| {
                                let before = doc.view.grid().scroll();
                                doc.view.grid_mut().scroll_to_fraction(
                                    info.nTrackPos as f32 / SCROLL_RANGE as f32,
                                );
                                doc.view.grid().scroll() != before
                            })
                            == Some(true)
                    }
                    _ => false,
                };
                if moved {
                    shell.sync_scrollbar(hwnd);
                    unsafe {
                        let _ = InvalidateRect(hwnd, None, false);
                    }
                }
            });
            LRESULT(0)
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

/// Opens a source on a worker thread.
///
/// Off the window thread for the same reason the device is: a multi-GB file indexed on the message
/// loop would undo the two-stage paint `experiments/g3-d3d11` measured at 13.1 ms. It matters more
/// for a pipe, where the producer decides how long the open takes and may never finish at all.
fn spawn_open(
    open: impl FnOnce() -> std::result::Result<Document, String> + Send + 'static,
) -> Receiver<std::result::Result<Document, String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(open());
    });
    rx
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
    // §13.2: spill files are "reaped on next launch if orphaned". Before anything creates one, and
    // cheap — it lists `%TEMP%` once and only touches names this product produces.
    let _ = reap_orphans();

    let reading = match std::env::args_os().nth(1) {
        // `-` is the conventional name for the standard input stream, and `PLAN.md`'s M4
        // done-criterion spells the command out: `docker logs -f svc | tailhawk -`. It is not part
        // of §12.2's option surface — it is a *path* that by convention names the stream — so
        // honouring it now is not a guess at the flag design that lands at M8.
        Some(arg) if arg == "-" => Some(spawn_open(Document::from_pipe)),
        Some(arg) => Some(spawn_open(move || {
            Document::open(std::path::Path::new(&arg))
        })),
        // **No path, so look at the standard input handle** — §4.2. `FILE_TYPE_CHAR` is an
        // interactive console and §4.2 says "do not block": reading it would wait for a human to
        // type, which for a windowed application means a window that never appears.
        None if stdin_kind().readable() => Some(spawn_open(Document::from_pipe)),
        None => None,
    };

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
            frames: Frames::new(),
            last_double: None,
            file: None,
        });
    });

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            windows::core::w!("Tailhawk"),
            WS_OVERLAPPEDWINDOW | WS_VSCROLL,
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
    use tailhawk_core::Position;
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
        assert_eq!(doc.set.total_rows(), 5_000);
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

    /// **A copy is the file's own bytes, not a re-rendering of them** — `SPEC.md` §5.6.
    ///
    /// The clipboard call needs a window station and is not exercised here. What is, is the part
    /// that decides *what* gets copied, which is where content can be lost silently.
    ///
    /// The fixture carries `U+202E` — §13.4's Trojan Source character — and the selection that
    /// matters is the one whose **boundary lands on it**. A zero-width cluster occupies no column,
    /// so it shares one with the character after it; `byte_span` rounds outwards precisely so that
    /// selecting up to that column still copies the override. Selecting the whole row would include
    /// it trivially and prove nothing, which is what a first version of this test did.
    #[test]
    fn copying_a_selection_yields_the_files_own_bytes() {
        let path = std::env::temp_dir().join("tailhawk_copy_test.log");
        // Row 1 is `second `, U+202E, `hidden`, U+202C, ` line`. Written as escapes because rustc
        // rejects a bidi override in source (`text_direction_codepoint_in_literal`) — its own
        // Trojan Source defence, firing on a test about Trojan Source.
        std::fs::write(
            &path,
            "alpha beta gamma\nsecond \u{202E}hidden\u{202C} line\nthird\n",
        )
        .expect("write the fixture");
        let mut doc = Document::open(&path).expect("open");
        doc.lay_out((8.0, 10.0), (800, 200));

        // A whole row, as a triple-click takes it.
        doc.selection = Some(Selection::line(0));
        assert_eq!(doc.copy_text().as_deref(), Some("alpha beta gamma\n"));

        // A span inside one row. This row is ASCII, so its columns are its bytes.
        doc.selection = Some(Selection::stream(Position::new(0, 6), Position::new(0, 10)));
        assert_eq!(doc.copy_text().as_deref(), Some("beta"));

        // **The boundary case.** `second ` is columns 0..7, and the override sits at column 7 —
        // the same column as the `h` that follows it, because it is zero-width. Selecting 0..7 must
        // pull it in, or a copy has silently dropped an attacker-supplied character.
        doc.selection = Some(Selection::stream(Position::new(1, 0), Position::new(1, 7)));
        let copied = doc.copy_text().expect("row 1");
        assert!(
            copied.contains('\u{202E}'),
            "the bidi override was laundered out of the copy: {copied:?}"
        );

        // Two rows: the first carries its line break, the last does not.
        doc.selection = Some(Selection::stream(Position::new(0, 0), Position::new(1, 6)));
        assert_eq!(doc.copy_text().as_deref(), Some("alpha beta gamma\nsecond"));

        // A caret copies nothing at all — not an empty string, nothing.
        doc.selection = Some(Selection::at(Position::new(0, 3)));
        assert_eq!(doc.copy_text(), None);
        doc.selection = None;
        assert_eq!(doc.copy_text(), None);

        let _ = std::fs::remove_file(&path);
    }

    /// The selection reaches the painter as **columns**, per §3.3, and only for the rows it covers.
    #[test]
    fn the_painter_is_told_which_columns_are_selected() {
        let path = std::env::temp_dir().join("tailhawk_rowsel_test.log");
        std::fs::write(&path, "alpha beta gamma\nsecond line\nthird\n").expect("write");
        let mut doc = Document::open(&path).expect("open");
        doc.lay_out((8.0, 10.0), (800, 200));

        doc.selection = Some(Selection::stream(Position::new(0, 6), Position::new(1, 6)));

        // Row 0 runs from column 6 to the end of the line; `usize::MAX` is how `ToLineEnd` reaches
        // the painter, which needs no line length to tint to the right edge.
        assert_eq!(doc.row_selection(0), Some(6..usize::MAX));
        assert_eq!(doc.row_selection(1), Some(0..6));
        // Rows outside the selection are not tinted, and neither is one that is not on screen.
        assert_eq!(doc.row_selection(2), None);
        assert_eq!(doc.row_selection(99), None);

        // A caret selects no columns anywhere, so nothing is tinted.
        doc.selection = Some(Selection::at(Position::new(0, 3)));
        assert_eq!(doc.row_selection(0), None);

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
