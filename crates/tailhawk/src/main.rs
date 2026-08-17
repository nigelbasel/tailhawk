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

use tailhawk_core::cell::CellModel;
use tailhawk_core::columns::{Layout, Presentation};
use tailhawk_core::detect::{self, Detection};
use tailhawk_core::filter::{Chip, Chips, Polarity};
use tailhawk_core::find::{self, Outcome, Running, Update};
use tailhawk_core::highlight::{Highlighter, Span};
use tailhawk_core::paint::{Colours, Painter};
use tailhawk_core::search::{Match, SearchOptions};
use tailhawk_core::semantic;
use tailhawk_core::set::LogSet;
use tailhawk_core::settings;
use tailhawk_core::sieve;
use tailhawk_core::stdin::{reap_orphans, stdin as stdin_kind, Pump, StreamEnd};
use tailhawk_core::template;
use tailhawk_core::widget::{Focus, Move, TextField};
use tailhawk_core::{
    background_rgb8, Renderer, RowEnd, RowSource, Selection, View, WindowHandle, CONTINUATION_INK,
    CURRENT_MATCH_BG, CURRENT_MATCH_INK, HEADER_INK, INK, MATCH_BG, RENDER_CAP_CELLS,
};
use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::{
    GlobalFree, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, InvalidateRect};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MK_SHIFT};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::Ime::{
    ImmGetCompositionStringW, ImmGetContext, ImmReleaseContext, ImmSetCompositionWindow, CFS_POINT,
    COMPOSITIONFORM, GCS_COMPSTR, GCS_CURSORPOS, GCS_RESULTSTR,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetDoubleClickTime, GetKeyState, ReleaseCapture, SetCapture, VK_A, VK_B, VK_BACK, VK_C,
    VK_CONTROL, VK_DELETE, VK_DOWN, VK_E, VK_END, VK_ESCAPE, VK_F, VK_F3, VK_HOME, VK_I, VK_L,
    VK_LEFT, VK_NEXT, VK_O, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP, VK_V,
    VK_W, VK_X, VK_Y, VK_Z,
};
use windows::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetScrollInfo, GetWindowPlacement, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW,
    SetTimer, SetWindowPos, SetWindowTextW, ShowWindow, SystemParametersInfoW, TranslateMessage,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, IDC_ARROW, MSG, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP,
    SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO,
    SIF_DISABLENOSCROLL, SIF_PAGE, SIF_POS, SIF_RANGE, SIF_TRACKPOS, SPI_GETWHEELSCROLLLINES,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOW, SW_SHOWMAXIMIZED, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    WHEEL_DELTA, WINDOWPLACEMENT, WINDOW_EX_STYLE, WM_CHAR, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_DROPFILES, WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION, WM_IME_STARTCOMPOSITION, WM_KEYDOWN,
    WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_PAINT, WM_SIZE, WM_TIMER, WM_VSCROLL, WNDCLASSW, WS_OVERLAPPEDWINDOW, WS_VSCROLL,
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
    /// The find state — the query, its matches, and the pass producing them. See [`Finder`].
    finder: Finder,
    /// §7.1's zero-config semantic layer, applied to visible rows beneath the search's matches.
    /// Built with the document, on the worker, because it compiles twenty regexes.
    highlighter: Highlighter,
    /// The filter state and the derived row space it makes — §7.3. See [`Filtering`].
    filtering: Filtering,
    /// The command bar: the find field, the chip row, the focus. See [`Chrome`].
    chrome: Chrome,
    /// The path this was opened from — the key for §12.4's per-file state. `None` for a pipe.
    path: Option<std::path::PathBuf>,
    /// The tab strip's labels and which is this document — set by the shell before each frame,
    /// because a document knows nothing of the others. Empty or one label: no strip.
    tab_strip: (Vec<String>, usize),
    /// The status bar's text — the title's status, set by the shell before each frame so the
    /// document can draw it in the footer band. The title keeps it too: the harnesses read it there.
    status: String,
    /// §8.1's per-tab change indication: this document grew while another tab was shown. Set by
    /// the shell on the follow tick, cleared when the tab is shown, drawn as a dot on its tab.
    unseen: bool,
    /// What §6.3's detector made of the newest member's head — the format, if one was accepted,
    /// the candidates if not. Run on the worker that opens the file, from the head sample only;
    /// §6.3's mid and tail samples are not taken yet.
    detection: Detection,
    /// V5: the column layout for the accepted format, sized on the head sample, or `None` for a
    /// file shown as it is written. See [`tailhawk_core::columns`].
    layout: Option<Layout>,
    /// `layout.header()`, built once at open. See [`Document::header_text`].
    header: Option<String>,
    /// The visible rows' presentations under `layout`, rebuilt by `lay_out` each frame — §7.1's
    /// visible-rows rule applied to columns. Keyed by **file** row, ascending.
    presented: Vec<(u64, Presentation)>,
    /// The first frame goes to the tail. **A tail tool opens at the end** — `SPEC.md` §6.1: "for a
    /// tailing tool the tail is what the user cares about" — and `Grid::is_following` is derived from
    /// being at the bottom, so this is also what makes a freshly opened file follow. Cleared once
    /// `lay_out` has real metrics to scroll with; the view is built with placeholder metrics and
    /// replaced before the first frame draws.
    open_at_tail: bool,
    /// Whether the producer had closed its end as of the last tick.
    ///
    /// **A remembered edge, not a query.** The title only redraws when a tick reports a change, and
    /// end-of-stream changes nothing about the file — so without this the window that has stopped
    /// growing goes on saying "reading stdin", which is what a hung window looks like.
    stream_done: bool,
}

/// **Every row the painter, the selection and the clipboard name is a *view* row**, and the
/// filter's derived row space (§7.3) is what turns it into a file row — here, in one place, so
/// nothing below this knows whether a filter is on. With no chips the two are the same number.
impl RowSource for Document {
    /// Under a column layout the painter draws the row's *presentation* — its fields aligned —
    /// built for the visible rows in `lay_out`; otherwise the raw line.
    fn row_text(&self, row: u64) -> Option<&str> {
        let file_row = self.filtering.file_row(row)?;
        match self.presentation(file_row) {
            Some(p) => Some(&p.text),
            None => self.set.row_text(file_row),
        }
    }

    /// A presented row has no anchors — they describe the raw line — and every lookup accepts
    /// none; it costs a walk over one screen row.
    fn row_anchors(&self, row: u64) -> &tailhawk_core::cell::ColumnAnchors {
        match self.filtering.file_row(row) {
            Some(file_row) if self.presentation(file_row).is_none() => {
                self.set.row_anchors(file_row)
            }
            _ => tailhawk_core::cell::ColumnAnchors::none_ref(),
        }
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

    /// §7.1's colours: the search's matches on top, the semantic catalogue beneath. **Visible rows
    /// only**, which is what §7.1 requires and what being called from the painter's row loop
    /// delivers for free.
    ///
    /// **The matches go in first**, so a hit the user asked for is never hidden by a timestamp
    /// colour, and the catalogue fills in around them — `Highlighter::beneath` exists for this
    /// call. A row whose text is not in memory yet gets the matches and nothing else, which is
    /// also what it gets drawn as.
    fn header(&self) -> Option<&str> {
        self.header_text()
    }

    /// The command bar: `▸ [find] ▼ [+chip] [−chip] [add filter…]     · format`.
    ///
    /// Everything is placed in cells so it lines up with the rows. Fields are filled rectangles
    /// with their text over them; the focused one carries a caret and, while an IME composes, a
    /// mark under the composition. Chips are their text on a chip fill; a click on one removes it
    /// (`UI-DESIGN.md` §5's toggle and reorder are not here yet, and this is the honest first
    /// affordance). What was drawn where is remembered for the click.
    fn draw_chrome(&self, painter: &mut Painter, view: &View) {
        let cell_w = painter.cell_width();
        let row_h = painter.row_height();
        let band = view.chrome_px();
        let width = view.hgrid().viewport_px();
        let cells = view.cells();
        let mut hits = self.chrome.hits.borrow_mut();
        hits.clear();

        painter.fill(0.0, 0.0, width, band, CHROME_BG);

        // The tab strip, when there is more than one tab: each file's name on its own fill, the
        // shown one lighter, and a click on one shows it (`Hit::Tab`). Above the bar.
        let (labels, active) = &self.tab_strip;
        let strip = if labels.len() > 1 {
            let strip_h = Chrome::strip_height(row_h);
            let ty = ((strip_h - row_h) / 2.0).floor();
            let mut tx = cell_w * 0.5;
            for (i, label) in labels.iter().enumerate() {
                let w = cells.cell_count(label) as f32 * cell_w;
                let bg = if i == *active { TAB_ACTIVE_BG } else { TAB_BG };
                painter.fill(tx - 2.0, 1.0, w + cell_w * 2.0 + 4.0, strip_h - 2.0, bg);
                let ink = if i == *active { INK } else { HEADER_INK };
                let _ = painter.lay_out_at(view, tx + cell_w, ty, label, Colours::plain(ink));
                hits.push((tx..tx + w + cell_w * 2.0, Hit::Tab(i)));
                tx += w + cell_w * 3.0;
            }
            strip_h
        } else {
            0.0
        };
        let text_y = strip + ((band - strip - row_h) / 2.0).floor();
        let mut x = cell_w * 0.5;

        // ▸ and the find field. (`UI-DESIGN.md` §2.1's ▸ is not in Cascadia Mono, and the painter
        // has one face until fallback lands; a placeholder box is worse than a plain marker.)
        let _ = painter.lay_out_at(view, x, text_y, "▸", Colours::plain(HEADER_INK));
        x += cell_w * 2.0;
        let find_w = FIND_CELLS as f32 * cell_w;
        let find_focused = self.chrome.focus == Focus::Find;
        painter.fill(
            x - 2.0,
            text_y - 2.0,
            find_w + 4.0,
            row_h + 4.0,
            if find_focused {
                FIELD_BG_FOCUSED
            } else {
                FIELD_BG
            },
        );
        hits.push((x..x + find_w, Hit::Find));
        let find_origin = x;
        draw_field(
            painter,
            view,
            cells,
            &self.chrome.find,
            find_focused,
            x,
            text_y,
            FIND_CELLS,
            "search (Ctrl+F)",
        );
        x += find_w + cell_w * 2.0;

        // ▼ and the chips.
        let _ = painter.lay_out_at(view, x, text_y, "▼", Colours::plain(HEADER_INK));
        x += cell_w * 2.0;
        for (i, chip) in self.filtering.chips.chips.iter().enumerate() {
            let sign = match chip.polarity {
                Polarity::Include => "+",
                Polarity::Exclude => "−",
            };
            let label = format!("{sign}{}", chip.source);
            let w = cells.cell_count(&label) as f32 * cell_w;
            // The body toggles, the `×` after it removes; a disabled chip is drawn dim on its fill.
            let close_w = cell_w * 2.0;
            let bg = match (chip.polarity, chip.enabled) {
                (Polarity::Include, true) => CHIP_INCLUDE_BG,
                (Polarity::Exclude, true) => CHIP_EXCLUDE_BG,
                (_, false) => FIELD_BG,
            };
            painter.fill(x - 2.0, text_y - 2.0, w + close_w + 4.0, row_h + 4.0, bg);
            let ink = if chip.enabled { INK } else { FIELD_HINT };
            let _ = painter.lay_out_at(view, x, text_y, &label, Colours::plain(ink));
            let _ = painter.lay_out_at(
                view,
                x + w + cell_w * 0.5,
                text_y,
                "×",
                Colours::plain(FIELD_HINT),
            );
            hits.push((x..x + w, Hit::Chip(i)));
            hits.push((x + w..x + w + close_w, Hit::ChipClose(i)));
            x += w + close_w + cell_w * 1.5;
        }
        // The new-chip field.
        let chip_w = CHIP_CELLS as f32 * cell_w;
        let chip_focused = self.chrome.focus == Focus::NewChip;
        painter.fill(
            x - 2.0,
            text_y - 2.0,
            chip_w + 4.0,
            row_h + 4.0,
            if chip_focused {
                FIELD_BG_FOCUSED
            } else {
                FIELD_BG
            },
        );
        hits.push((x..x + chip_w, Hit::NewChip));
        let chip_origin = x;
        let hint = match self.chrome.chip_polarity {
            Polarity::Include => "+ filter (Ctrl+L)",
            Polarity::Exclude => "− exclude (Ctrl+Shift+L)",
        };
        draw_field(
            painter,
            view,
            cells,
            &self.chrome.chip,
            chip_focused,
            x,
            text_y,
            CHIP_CELLS,
            hint,
        );
        self.chrome.origins.set((find_origin, chip_origin));

        // The format, at the right edge — §6.5's chip, as text for now.
        if let Some(text) = self.detection.describe() {
            let w = cells.cell_count(&text) as f32 * cell_w;
            let fx = width - w - cell_w;
            if fx > x + chip_w + cell_w {
                let _ = painter.lay_out_at(view, fx, text_y, &text, Colours::plain(HEADER_INK));
            }
        }

        // The status bar, at the bottom: what the title says, where a user looks. Cut from the
        // right if it is longer than the window; the front is the part that changes.
        let footer = view.footer_px();
        if footer > 0.0 && !self.status.is_empty() {
            let fy = view.height_px() - footer;
            painter.fill(0.0, fy, width, footer, CHROME_BG);
            let avail = ((width - cell_w) / cell_w).max(0.0) as usize;
            let shown = tailhawk_core::widget::fit_from_right(cells, &self.status, avail);
            let ty = fy + ((footer - row_h) / 2.0).floor();
            let _ = painter.lay_out_at(view, cell_w * 0.5, ty, shown, Colours::plain(HEADER_INK));
        }
    }

    fn row_spans(&self, row: u64, out: &mut Vec<Span>) {
        // Matches are file rows — the search snapshots the file, not the view.
        let Some(file_row) = self.filtering.file_row(row) else {
            out.clear();
            return;
        };
        self.finder.spans(file_row, out);
        match self.presentation(file_row) {
            // Matches are byte ranges in the raw line; the presentation carries them to where its
            // fields put those bytes, then the catalogue colours the text that is on screen.
            Some(p) => {
                let raw = std::mem::take(out);
                p.map(&raw, out);
                self.highlighter.beneath(&p.text, out);
                // §6.4: a continuation is "rendered dimmed and indented". Indented by the
                // presentation; dimmed here, beneath everything a rule or a match claimed.
                if p.continuation {
                    tailhawk_core::highlight::claim_beneath(
                        out,
                        Span {
                            start: 0,
                            end: p.text.len(),
                            fg: Some(CONTINUATION_INK),
                            bg: None,
                        },
                    );
                }
            }
            None => {
                if let Some(line) = self.set.row_text(file_row) {
                    self.highlighter.beneath(line, out);
                }
            }
        }
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
        let (detection, layout) = detect_set(&set, Some(path));
        let opened_from = Some(path.to_path_buf());

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
            finder: Finder::default(),
            highlighter: Highlighter::new(semantic::catalogue()),
            filtering: Filtering::default(),
            chrome: Chrome::default(),
            path: opened_from,
            tab_strip: (Vec::new(), 0),
            status: String::new(),
            unseen: false,
            detection,
            header: layout.as_ref().map(Layout::header),
            layout,
            presented: Vec::new(),
            open_at_tail: true,
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
        let (detection, layout) = detect_set(&set, None);
        let opened_from = None;
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
            finder: Finder::default(),
            highlighter: Highlighter::new(semantic::catalogue()),
            filtering: Filtering::default(),
            chrome: Chrome::default(),
            path: opened_from,
            tab_strip: (Vec::new(), 0),
            status: String::new(),
            unseen: false,
            detection,
            header: layout.as_ref().map(Layout::header),
            layout,
            presented: Vec::new(),
            open_at_tail: true,
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
        // The command bar always, the tab strip when there is more than one tab; one row of header
        // when there are columns. Set after the viewport, which is what all three are subtracted from.
        let strip = if self.tab_strip.0.len() > 1 {
            Chrome::strip_height(row_h)
        } else {
            0.0
        };
        self.view.set_chrome_px(strip + Chrome::height(row_h));
        self.view.set_footer_px(Chrome::strip_height(row_h));
        self.view
            .set_header_px(if self.header.is_some() { row_h } else { 0.0 });
        {
            let rows = self.view_rows();
            self.view.grid_mut().set_total_rows(rows);
            if self.open_at_tail && rows > 0 {
                self.view.grid_mut().scroll_to_bottom();
                self.open_at_tail = false;
            }
        }

        // Across the whole set, not just the live member: §5.5b's scrollback reaches into files
        // whose widest line may be wider than anything the current one holds.
        let extent = self.set.extent();
        let padding = self.layout.as_ref().map_or(0, Layout::extra_cells) as u64;
        let columns = extent
            .exact_cells(self.set.charset())
            .unwrap_or_else(|| extent.max_line_bytes())
            .saturating_add(padding)
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
        let file_rows: Vec<u64> = visible
            .iter()
            .filter_map(|&r| self.filtering.file_row(r))
            .collect();
        // A format whose message is the next line (MEL Simple) has that line pulled into the
        // record's row while continuations are collapsed — the assembly §6.4 asks for. That needs
        // the successors fetched too, one more read per visible row; without collapse the message
        // is on screen anyway, one row down.
        let assemble = self
            .layout
            .as_ref()
            .is_some_and(|l| l.format.body_next_line && self.filtering.records_only);
        if assemble {
            let mut with_next: Vec<u64> = file_rows
                .iter()
                .flat_map(|&r| [r, r + 1])
                .filter(|&r| r < self.set.total_rows())
                .collect();
            with_next.dedup();
            let _ = self.set.fetch_rows(&with_next, anchored);
        } else if self.filtering.active() {
            // The visible view rows are survivors from anywhere in the file: a scattered fetch.
            let _ = self.set.fetch_rows(&file_rows, anchored);
        } else {
            let _ = self.set.fetch(first, count, anchored);
        }
        // V5: the visible rows' presentations, rebuilt every frame from the raw text just fetched.
        // Fifty parses a frame; §7.1's visible-rows rule, applied to columns.
        self.presented.clear();
        if let Some(layout) = &self.layout {
            for file_row in file_rows {
                if let Some(raw) = self.set.row_text(file_row) {
                    let next = if assemble {
                        self.set.row_text(file_row + 1)
                    } else {
                        None
                    };
                    self.presented
                        .push((file_row, layout.present_record(raw, next)));
                }
            }
        }
    }

    /// V5's column header, when there is a layout — `RowSource::header`, drawn in the band the
    /// view reserves. Cached because the painter asks every frame and the answer never changes.
    fn header_text(&self) -> Option<&str> {
        self.header.as_deref()
    }

    /// The presentation of a file row, if it is one of the visible rows `lay_out` presented.
    fn presentation(&self, file_row: u64) -> Option<&Presentation> {
        self.presented
            .binary_search_by_key(&file_row, |(r, _)| *r)
            .ok()
            .map(|i| &self.presented[i].1)
    }

    /// How this file is being looked at, for §12.4's per-file state — `None` for a pipe, which has
    /// no path to key by. Chips carry their polarity as a leading `+` or `-`.
    fn file_state(&self) -> Option<settings::FileState> {
        let path = self.path.as_ref()?.to_string_lossy().into_owned();
        let chips = self
            .filtering
            .chips
            .chips
            .iter()
            .map(|c| {
                let sign = match c.polarity {
                    Polarity::Include => '+',
                    Polarity::Exclude => '-',
                };
                format!("{sign}{}", c.source)
            })
            .collect();
        Some(settings::FileState {
            path,
            chips,
            collapse: self.filtering.records_only,
        })
    }

    /// Restores a remembered view: the chips and the collapse, then one pass. A chip that no
    /// longer parses is dropped quietly — the file is being opened, not edited.
    fn apply_state(&mut self, state: &settings::FileState) {
        for chip in &state.chips {
            let (polarity, text) = match chip.chars().next() {
                Some('-') => (Polarity::Exclude, &chip[1..]),
                Some('+') => (Polarity::Include, &chip[1..]),
                _ => (Polarity::Include, chip.as_str()),
            };
            if let Ok(chip) = Chip::parse(text, polarity) {
                self.filtering.chips.chips.push(chip);
            }
        }
        self.filtering.records_only = state.collapse && self.detection.accepted.is_some();
        self.filtering.error = None;
        if self.filtering.active() {
            self.filtering.clear_results();
            self.refilter();
        }
    }

    /// Rows in the view's row space: the survivors while a filter is on, the file's rows otherwise.
    fn view_rows(&self) -> u64 {
        if self.filtering.active() {
            self.filtering.kept.len() as u64
        } else {
            self.set.total_rows()
        }
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
        // **The find state goes first**, because it is the part that changes while the user is
        // watching and the part a truncated title must not lose. Everything after it is the
        // document, which is what the window said before there was a search.
        let find = match self.finder.describe() {
            Some(text) => format!("{text} — "),
            None => String::new(),
        };
        let filter = match self.filtering.describe(self.set.total_rows()) {
            Some(text) => format!("{text} — "),
            None => String::new(),
        };
        let reveal = if self.view.cells().reveal_invisibles {
            "¶ revealing invisibles — "
        } else {
            ""
        };
        let format = match self.detection.describe() {
            Some(text) => format!(" · {text}"),
            None => String::new(),
        };
        // §12: scrolling up pauses following, and the affordance to resume must be visible — the
        // "single most-wanted behaviour in every tail tool", and getting it wrong is very visible.
        // A pipe that has finished is not paused, it is done.
        let following = if self.stream_done {
            ""
        } else if self.view.grid().is_following() {
            "● following — "
        } else {
            "‖ paused · Ctrl+End to follow — "
        };
        format!(
            "{following}{find}{filter}{reveal}{}: {}{flag}{source}{format}, {} lines, {} bytes",
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
            // **And the matches, for the same reason and more sharply.** A match is a row number
            // plus a byte range inside it; a truncated member's rows are different bytes at the
            // same numbers and a retirement renumbers everything after it. Keeping them would
            // paint highlight over text that was never matched and send `F3` to the wrong line —
            // and unlike a stale selection, that is a wrong answer rather than a wrong tint.
            //
            // A *roll* does not clear them: §5.5b appends the new member at the end of the row
            // space, so every existing row keeps its number. The results are still a snapshot and
            // still do not cover the new bytes, which is what "searched N lines" in the title says.
            self.finder.clear();
            // And the survivors: a filtered view over renumbered rows would show the wrong lines.
            // The chips stay, and the pass restarts over what the file now is.
            self.filtering.clear_results();
            self.refilter();
        }

        // Growth is sieved on the worker as it arrives — see `Filtering::covered`.
        self.refilter();
        {
            let rows = self.view_rows();
            self.view.grid_mut().set_total_rows(rows);
        }
        if was_following {
            // A tail that rolls keeps tailing. §5.5b wants a separator row at the boundary too,
            // which is a rendering feature and is not done — `LogSet::locate` reports where one
            // goes, and `HANDOFF.md` records that nothing draws it.
            self.view.grid_mut().scroll_to_bottom();
        }
        true
    }

    /// Starts a search for the typed query, over a snapshot of the set.
    ///
    /// **The snapshot is taken here and the pass runs on a worker**, because §7.4's own figure for a
    /// full pass over 10 GB is 9.93 s and §11.3 forbids blocking a frame. See [`tailhawk_core::find`]
    /// for what the worker is given and why it cannot move under it.
    fn find(&mut self) {
        self.finder.clear();
        if self.finder.query.is_empty() {
            return;
        }
        // Where the user is now, so the first match shown is the next one rather than the first in
        // the file — see `Finder::first_worth_showing`.
        self.finder.from_row = self
            .filtering
            .file_row(self.view.grid().scroll().row)
            .unwrap_or(0);
        match find::start(
            &self.finder.query,
            true,
            self.set.snapshot(),
            SearchOptions::default(),
        ) {
            Ok(running) => self.finder.running = Some(running),
            // A pattern the engine refused. It reaches the title rather than a dialog, because the
            // window is not modal and the user is still looking at what they typed.
            Err(e) => self.finder.error = Some(e.to_string()),
        }
    }

    /// Adds a chip and starts the pass over. A chip that does not parse is held in the title, as a
    /// bad pattern is; the chips already there stand.
    fn add_chip(&mut self, text: &str, polarity: Polarity) {
        if text.trim().is_empty() {
            return;
        }
        match Chip::parse(text, polarity) {
            Ok(chip) => {
                self.filtering.error = None;
                self.filtering.chips.chips.push(chip);
                self.filtering.clear_results();
                self.refilter();
            }
            Err(e) => self.filtering.error = Some(format!("{text}: {e}")),
        }
    }

    /// Drops every chip and the survivors with them: the unfiltered view.
    fn clear_filter(&mut self) {
        self.filtering.chips.chips.clear();
        self.filtering.error = None;
        self.filtering.clear_results();
        {
            let rows = self.view_rows();
            self.view.grid_mut().set_total_rows(rows);
        }
    }

    /// Starts a pass over whatever rows no pass has covered yet — all of them after a chip
    /// changes, only the growth after a follow tick. **Never on the window thread**: §7.3 says a
    /// filter change is a full-file pass, and §11.3 says a frame is not the place for one.
    fn refilter(&mut self) {
        if !self.filtering.active() || self.filtering.running.is_some() {
            return;
        }
        let total = self.set.total_rows();
        let from = self.filtering.covered;
        if from >= total {
            return;
        }
        match sieve::start(
            self.filtering.chips.clone(),
            self.detection.accepted,
            self.filtering.records_only,
            self.set.snapshot(),
            from,
            total,
            SearchOptions::default(),
        ) {
            Ok(running) => {
                self.filtering.running = Some(running);
                self.filtering.covered = total;
                self.filtering.outcome = None;
            }
            Err(e) => self.filtering.error = Some(e.to_string()),
        }
    }

    /// Collects what the filter worker has reported. Reports whether the view changed.
    ///
    /// **Following stays following.** The pass streams survivors out of order, so the bottom of
    /// the view moves as chunks land; a reader who was at the bottom is kept there, exactly as
    /// `poll_follow` keeps them there when the file grows.
    fn poll_filter(&mut self) -> bool {
        let Some(running) = self.filtering.running.as_ref() else {
            return false;
        };
        let updates: Vec<sieve::Update> = running.drain().collect();
        if updates.is_empty() {
            return false;
        }
        let was_following = self.view.grid().is_following();
        let mut finished = false;
        for update in updates {
            match update {
                sieve::Update::Chunk(kept) => {
                    self.filtering.scanned += kept.scanned;
                    self.filtering.absorb(kept.rows);
                }
                sieve::Update::Finished(outcome) => {
                    self.filtering.outcome = Some(outcome);
                    finished = true;
                }
            }
        }
        if finished {
            self.filtering.running = None;
            // Growth that arrived while the pass ran is the next pass.
            self.refilter();
        }
        {
            let rows = self.view_rows();
            self.view.grid_mut().set_total_rows(rows);
        }
        if was_following {
            self.view.grid_mut().scroll_to_bottom();
        }
        true
    }

    /// Steps to the next or previous match and puts it on screen. Reports whether it moved.
    fn find_step(&mut self, forward: bool) -> bool {
        let Some(row) = self.finder.step(forward) else {
            return false;
        };
        self.show_row(row);
        true
    }

    /// Collects what the search worker has reported, showing the first match worth showing.
    ///
    /// **The scrolling lives here rather than in the shell**, per §3.1: the shell decides what a
    /// message means and the document decides what moving means. It is also the only way the join
    /// is testable — a shell method needs a window, and "did the view follow the match" is exactly
    /// the assertion that would otherwise be made by hand, once.
    fn poll_find(&mut self) -> bool {
        let (changed, show) = self.finder.poll();
        if let Some(row) = show {
            self.show_row(row);
        }
        changed
    }

    /// Puts `row` on screen, leaving the view alone if it is already there.
    ///
    /// **A match already visible must not move the view.** Stepping through four hits on one screen
    /// should light each one in turn, not scroll four times — and a match that *is* off screen lands
    /// a third of a page down rather than on the top edge, so there is context above it.
    fn show_row(&mut self, row: u64) {
        // A file row; the view scrolls in its own row space. A hidden match lands on the next
        // survivor, which keeps `F3` moving through the file rather than stalling on a row that
        // is not there to be shown.
        let row = self.filtering.view_row(row);
        let grid = self.view.grid();
        let first = grid.scroll().row;
        if row >= first && row < first.saturating_add(grid.page_rows()) {
            return;
        }
        let above = grid.page_rows() / 3;
        self.view
            .grid_mut()
            .scroll_to_row(row.saturating_sub(above));
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
                self.selection = Some(match self.row_text(at.row) {
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
            let Some(line) = self.row_text(row) else {
                continue;
            };
            // A whole row copies its **raw** bytes even under columns — §12: `Ctrl+C` is raw. A
            // part of a presented row copies what is on screen, which is the only thing its cell
            // range names.
            let whole = sel
                .row_span(row)
                .is_some_and(|s| s.start_cell == 0 && s.end == RowEnd::ToLineEnd);
            let raw = self
                .filtering
                .file_row(row)
                .and_then(|f| self.set.row_text(f));
            match (whole, raw) {
                (true, Some(raw)) => out.push_str(raw),
                _ => {
                    if let Some(bytes) = sel.byte_range(self.view.cells(), row, line) {
                        out.push_str(&line[bytes]);
                    }
                }
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

/// The find state — `UI-DESIGN.md` §12's `Ctrl+F`, `F3` and `Shift+F3`.
///
/// ## ⚠ There is no find *bar*, and that is a disclosed deviation
///
/// `UI-DESIGN.md` §2.1 puts search in a command bar — "a single field … no modal find dialog" — and
/// there is no field here because there is no **widget layer**: V14 is M7's first item and §12's
/// text input depends on it. So the query is typed into the window and echoed in the title bar,
/// which is where every other piece of live state has gone for the same reason (§5.5b's set
/// description, §4.2's spill path, the frame instrument).
///
/// It is recorded in `CLEANROOM.md` as a deviation rather than presented as the design. What it does
/// **not** do is prejudge the widget layer: everything below the keystroke — the query string, the
/// match list, the stepping, the spans — is what a real field would drive, and only the two `WM_CHAR`
/// arms would be deleted.
///
/// ## The search is case-insensitive, and that is a decision
///
/// §7.2 gives the grammar a `/pattern/i` flag and there is nowhere to set it, so the default is the
/// whole behaviour. **Insensitive**, because the two failures are not symmetric: a user who typed
/// `error` and wanted only `error` sees hits they did not want, which is visible and correctable
/// from the screen, while a user who typed `error` against a log that says `ERROR` sees *nothing*
/// and concludes the search is broken. `(?-i)` in the pattern is the escape hatch until there is a
/// toggle.
#[derive(Default)]
struct Finder {
    /// The query the current results are for — what the find field held when `Enter` was pressed.
    /// The field itself lives in [`Chrome`]; this is what the title and the pass use.
    query: String,
    /// **Sorted by `(line, start)` at all times**, which is a property maintained on insertion
    /// rather than restored by sorting — see [`Finder::absorb`].
    matches: Vec<Match>,
    /// Index into [`matches`](Self::matches) of the match `F3` last stepped to.
    current: Option<usize>,
    running: Option<Running>,
    scanned: u64,
    truncated: u64,
    outcome: Option<Outcome>,
    /// A pattern the engine refused. Held rather than shown once, because the window is not modal
    /// and the user is still looking at what they typed.
    error: Option<String>,
    /// The row the search started from, so the first match reported at or after it is the one to
    /// show. See [`Finder::first_worth_showing`].
    from_row: u64,
    /// Whether the view has already been moved for this search.
    jumped: bool,
}

impl Finder {
    /// Forgets the results, keeping the query. Cancels a running pass by dropping it.
    fn clear(&mut self) {
        self.running = None;
        self.matches.clear();
        self.current = None;
        self.scanned = 0;
        self.truncated = 0;
        self.outcome = None;
        self.error = None;
        self.jumped = false;
    }

    /// Adds one chunk's matches, keeping the list sorted **without sorting it**.
    ///
    /// §7.4 chunks by line and chunks are disjoint runs of lines, so every match in a chunk belongs
    /// in one contiguous slot of the ordered list — a `partition_point` finds it and a splice puts
    /// them there. Within a chunk they already arrive in order, because `search_lines` walks lines
    /// forwards.
    ///
    /// **The alternative was sorting after each drain, and it does not fit in a frame.** A 10 GB
    /// pass streams up to 100,000 matches; re-sorting that on every 100 ms tick is ~10 ms of window
    /// thread against a 16.67 ms budget, spent repeatedly to re-derive an order that was never lost.
    fn absorb(&mut self, matches: Vec<Match>) {
        let Some(first) = matches.first().copied() else {
            return;
        };
        let at = self
            .matches
            .partition_point(|m| (m.line, m.start) < (first.line, first.start));
        // A step that had a match under it keeps pointing at the same one: an insertion before it
        // shifts its index, and not moving the cursor would silently walk `F3` backwards.
        if let Some(current) = self.current.as_mut() {
            if *current >= at {
                *current += matches.len();
            }
        }
        self.matches.splice(at..at, matches);
    }

    /// The match a fresh search should scroll to: the first at or after where the user was.
    ///
    /// **`F3` from the top of a 10 GB file should not scroll to row 4** if the user is reading row
    /// 40 million. Wrapping to the start when there is nothing below is the other half of the same
    /// rule, and it is what every editor does.
    fn first_worth_showing(&self) -> Option<usize> {
        let at = self.matches.partition_point(|m| m.line < self.from_row);
        if at < self.matches.len() {
            Some(at)
        } else {
            (!self.matches.is_empty()).then_some(0)
        }
    }

    /// Steps to the next or previous match, wrapping. Returns the row to show.
    fn step(&mut self, forward: bool) -> Option<u64> {
        if self.matches.is_empty() {
            return None;
        }
        let next = match (self.current, forward) {
            (None, _) => self.first_worth_showing()?,
            (Some(at), true) => (at + 1) % self.matches.len(),
            (Some(0), false) => self.matches.len() - 1,
            (Some(at), false) => at - 1,
        };
        self.current = Some(next);
        Some(self.matches[next].line)
    }

    /// The row's matches as spans. §7.1's colours, over §7.4's results.
    fn spans(&self, row: u64, out: &mut Vec<Span>) {
        out.clear();
        let from = self.matches.partition_point(|m| m.line < row);
        let current = self.current.unwrap_or(usize::MAX);
        for (at, m) in self.matches.iter().enumerate().skip(from) {
            if m.line != row {
                break;
            }
            let (bg, fg) = if at == current {
                (CURRENT_MATCH_BG, Some(CURRENT_MATCH_INK))
            } else {
                (MATCH_BG, None)
            };
            out.push(Span {
                start: m.start,
                end: m.end,
                fg,
                bg: Some(bg),
            });
        }
    }

    /// Collects whatever the worker has reported. Returns the row to scroll to, if this is the
    /// first news of a match worth showing.
    ///
    /// **The view is moved once per search and never again**, however many chunks arrive afterwards.
    /// Chunks come back out of order (§7.4), so an earlier match routinely lands *after* a later
    /// one — re-jumping to whichever is now the best answer would drag the window around under a
    /// user who is already reading the result they were given.
    fn poll(&mut self) -> (bool, Option<u64>) {
        let Some(running) = self.running.as_ref() else {
            return (false, None);
        };
        let updates: Vec<Update> = running.drain().collect();
        if updates.is_empty() {
            return (false, None);
        }
        let mut finished = false;
        for update in updates {
            match update {
                Update::Chunk(found) => {
                    self.scanned += found.scanned;
                    self.truncated += found.truncated;
                    self.absorb(found.matches);
                }
                Update::Finished(outcome) => {
                    self.outcome = Some(outcome);
                    finished = true;
                }
            }
        }
        if finished {
            self.running = None;
        }
        let show = match (self.jumped, self.first_worth_showing()) {
            (false, Some(at)) => {
                self.jumped = true;
                self.current = Some(at);
                Some(self.matches[at].line)
            }
            _ => None,
        };
        (true, show)
    }

    /// The title's find fragment, or `None` when no search is in play.
    ///
    /// Everything §7.4 obliges a search to disclose is here: that a pass is still running, that it
    /// was capped, that lines were too slow to search, and — the one a user would otherwise read as
    /// "no matches" — that the pattern did not compile.
    fn describe(&self) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("▸ {} — {error}", self.query));
        }
        if self.query.is_empty() {
            return None;
        }
        let mut text = format!("▸ {}", self.query);
        match (self.current, self.matches.len()) {
            (_, 0) if self.running.is_some() => text.push_str(" — searching…"),
            (_, 0) => text.push_str(" — no matches"),
            (Some(at), n) => text.push_str(&format!(" — {} of {n}", at + 1)),
            (None, n) => text.push_str(&format!(" — {n} matches")),
        }
        if self.running.is_some() && !self.matches.is_empty() {
            text.push_str(&format!(", scanning ({} lines)", self.scanned));
        }
        match self.outcome {
            // §7.4's cap: there are matches that were never reported, so the count is a floor.
            Some(Outcome::Capped) => text.push_str(" — capped, there are more"),
            Some(Outcome::Cancelled) => text.push_str(" — cancelled"),
            Some(Outcome::Failed(ref why)) => text.push_str(&format!(" — read failed: {why}")),
            _ => {}
        }
        if self.truncated > 0 {
            // §7.4's "pattern too slow, truncated", counted rather than hidden.
            text.push_str(&format!(", {} lines too slow to search", self.truncated));
        }
        Some(text)
    }
}

/// §6.3's detection over the newest member's head. On the worker that opens the file, so the
/// 150 ms open budget is not the window thread's problem; a set with no members detects nothing.
/// §6.5's config scan (E11) rides along: templates found in `appsettings*.json`, `nlog.config`
/// or `log4net.config` beside `path` are compiled and scored with the catalogue. A path of `None`
/// — a pipe — has nothing beside it.
fn detect_set(set: &LogSet, path: Option<&std::path::Path>) -> (Detection, Option<Layout>) {
    let lines = match set.snapshot().last() {
        Some(newest) => detect::head_lines(&*newest.file, newest.charset),
        None => Vec::new(),
    };
    let templates: Vec<&'static tailhawk_core::format::Format> = path
        .map(template::scan)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|found| {
            let origin = found
                .source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            template::compile(found.language, &found.template, &origin).ok()
        })
        .collect();
    let detection = detect::detect_with(&lines, &templates);
    let layout = detection
        .accepted
        .map(|format| Layout::from_sample(format, &lines));
    (detection, layout)
}

/// Takes one `WM_CHAR` code unit into a typed field. Returns whether it was wanted.
///
/// **Surrogate pairs are joined here.** `WM_CHAR` delivers a non-BMP character as two messages,
/// and pushing each on its own puts two replacement characters into the text — a search for an
/// emoji that cannot match one, failing as "not found" rather than as anything a user could act
/// on. Logs carry emoji: every one of this project's own commit messages could. Shared by the find
/// query and the filter chip, which are the two things typed into the window until M7's fields.
fn push_typed_unit(text: &mut String, pending_high: &mut Option<u16>, unit: u16) -> bool {
    // Control characters arrive here too — `Ctrl+F` itself is 0x06 and `Enter` is 0x0D — and
    // every one of them is either handled as a key or is not wanted in a query.
    if unit < 0x20 || unit == 0x7F {
        return false;
    }
    match (pending_high.take(), unit) {
        (_, high) if (0xD800..0xDC00).contains(&high) => {
            *pending_high = Some(high);
        }
        (Some(high), low) if (0xDC00..0xE000).contains(&low) => {
            text.extend(char::decode_utf16([high, low]).map(|c| c.unwrap_or('\u{FFFD}')));
        }
        // **A high surrogate whose partner never came becomes U+FFFD rather than being
        // dropped**, because a character that vanishes as it is typed is indistinguishable from
        // a keyboard that missed it. `take` above has already removed it, so without this arm
        // passing it on it would go nowhere at all.
        (orphan, unit) => text.extend(
            char::decode_utf16(orphan.into_iter().chain([unit])).map(|c| c.unwrap_or('\u{FFFD}')),
        ),
    }
    true
}

/// The filter state — §7.3's in-place hide, driven the way [`Finder`] is until M7's chip row.
///
/// **This is the derived row space.** When `chips` is non-empty the grid counts `kept.len()` rows,
/// view row *k* is file row `kept[k]`, and everything the painter asks for is mapped through it.
/// `kept` is filled by [`tailhawk_core::sieve`] on a worker and maintained sorted **without
/// sorting**, exactly as [`Finder::absorb`] maintains the match list and for the same reason.
///
/// A chip is typed into the window — `Ctrl+L` for an include, `Ctrl+Shift+L` for an exclude,
/// `Enter` to add it — because V14's text field is M7; the title shows the chips as it shows the
/// query. There is no chip *editing*: `Esc` clears them all, which is the affordance one key can
/// honestly give.
#[derive(Default)]
struct Filtering {
    chips: Chips,
    /// §6.4's collapse: only first lines are rows, continuations hidden. A row space like the
    /// chips', sieved by the same pass, so the two compose. Meaningless without a format.
    records_only: bool,
    /// The file rows that survive, ascending — the view's row space while `chips` is non-empty.
    kept: Vec<u64>,
    running: Option<sieve::Running>,
    /// File rows `[0, covered)` have been given to a pass; anything past it is growth still to
    /// sieve, and [`Document::poll_filter`] starts the next pass over it when the current one ends.
    covered: u64,
    scanned: u64,
    outcome: Option<Outcome>,
    /// A chip that did not parse. Held, like the finder's, because the window is not modal.
    error: Option<String>,
}

impl Filtering {
    fn active(&self) -> bool {
        !self.chips.chips.is_empty() || self.records_only
    }

    /// Forgets the survivors and stops the pass, keeping the chips — what a truncate needs.
    fn clear_results(&mut self) {
        self.running = None;
        self.kept.clear();
        self.covered = 0;
        self.scanned = 0;
        self.outcome = None;
    }

    /// The view row a file row lands on: its own slot if it survived, otherwise the slot of the
    /// next survivor — which is where a hidden match, stepped to, puts the view.
    fn view_row(&self, file_row: u64) -> u64 {
        if !self.active() {
            return file_row;
        }
        self.kept.partition_point(|&r| r < file_row) as u64
    }

    fn file_row(&self, view_row: u64) -> Option<u64> {
        if !self.active() {
            return Some(view_row);
        }
        self.kept.get(usize::try_from(view_row).ok()?).copied()
    }

    /// Adds one chunk's survivors, in their slot, without sorting the list.
    fn absorb(&mut self, rows: Vec<u64>) {
        let Some(&first) = rows.first() else {
            return;
        };
        let at = self.kept.partition_point(|&r| r < first);
        self.kept.splice(at..at, rows);
    }

    /// The title's filter fragment, or `None` when no filter is in play.
    fn describe(&self, total_rows: u64) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("▼ {error}"));
        }
        let mut text = String::new();
        if self.records_only {
            text.push_str(" ▤ records");
        }
        for chip in &self.chips.chips {
            let sign = match chip.polarity {
                Polarity::Include => '+',
                Polarity::Exclude => '−',
            };
            text.push_str(&format!(" {sign}{}", chip.source));
        }
        if text.is_empty() {
            return None;
        }
        let mut text = format!("▼{text}");
        if self.active() {
            text.push_str(&format!(" · {} of {total_rows}", self.kept.len()));
            if self.running.is_some() {
                let pct = (self.scanned * 100)
                    .checked_div(total_rows)
                    .map_or(100, |p| p.min(100));
                text.push_str(&format!(" · scanning {pct}%"));
            }
            match self.outcome {
                Some(Outcome::Cancelled) => text.push_str(" · cancelled"),
                Some(Outcome::Failed(ref why)) => text.push_str(&format!(" · failed: {why}")),
                _ => {}
            }
        }
        Some(text)
    }
}

/// The command bar — V14 on V8's surface: the find field, the chip row and the new-chip field,
/// drawn by the painter in the band the view reserves above the header. `UI-DESIGN.md` §2.1.
///
/// The fields are [`TextField`]s; the keyboard goes to whichever [`Focus`] names, and to the grid
/// otherwise. What the bar shows is laid out in **cells** of the row grid — same shaper, same
/// cell model — so its text lines up with the columns beneath and a click resolves to a cell.
struct Chrome {
    find: TextField,
    chip: TextField,
    /// The polarity the next chip will have — `Ctrl+L` include, `Ctrl+Shift+L` exclude.
    chip_polarity: Polarity,
    focus: Focus,
    /// A high surrogate waiting for its low half, for `WM_CHAR` into whichever field has focus.
    pending_high: Option<u16>,
    /// What was drawn where, in viewport x pixels, so a click can be resolved. Filled by
    /// `draw_chrome` each frame; a `RefCell` because drawing takes `&self`.
    hits: std::cell::RefCell<Vec<(std::ops::Range<f32>, Hit)>>,
    /// The x each field's text starts at, for placing a caret from a click.
    origins: std::cell::Cell<(f32, f32)>,
}

/// What a click on the bar landed on.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Hit {
    Find,
    NewChip,
    /// A chip's body: toggles it.
    Chip(usize),
    /// A chip's `×`: removes it.
    ChipClose(usize),
    Tab(usize),
}

/// The bar's geometry, in cells. The find field is wide enough for a real query; the new-chip
/// field for a chip. Both scroll nothing — a query longer than the field is a rare event and the
/// caret stays visible because the text is cut from the *left* when it overflows.
const FIND_CELLS: usize = 36;
const CHIP_CELLS: usize = 24;

impl Default for Chrome {
    fn default() -> Self {
        Self {
            find: TextField::default(),
            chip: TextField::default(),
            chip_polarity: Polarity::Include,
            focus: Focus::Grid,
            pending_high: None,
            hits: std::cell::RefCell::new(Vec::new()),
            origins: std::cell::Cell::new((0.0, 0.0)),
        }
    }
}

impl Chrome {
    /// The field that has the keyboard, if one does.
    fn focused(&mut self) -> Option<&mut TextField> {
        match self.focus {
            Focus::Find => Some(&mut self.find),
            Focus::NewChip => Some(&mut self.chip),
            Focus::Grid => None,
        }
    }

    /// The bar's height for a row height: the row plus a little air, so the fields read as
    /// fields and not as a first row.
    fn height(row_h: f32) -> f32 {
        (row_h + 8.0).round()
    }

    /// The tab strip's height, when it is drawn.
    fn strip_height(row_h: f32) -> f32 {
        (row_h + 4.0).round()
    }

    /// The text of a field as it fits its width: cut from the left so the caret end is always on
    /// screen. Returns the text to draw and the byte offset the cut removed.
    fn fitted<'a>(cells: &CellModel, text: &'a str, width: usize) -> (&'a str, usize) {
        tailhawk_core::widget::fit_from_left(cells, text, width)
    }
}

/// The open documents and which one is shown — V7's tabs, without the strip's chrome yet.
///
/// `as_ref` / `as_mut` answer for the **active** document, which is what every handler means by
/// "the document"; the rest keep following in the background so switching to one is not a jump.
#[derive(Default)]
struct Tabs {
    docs: Vec<Document>,
    active: usize,
}

impl Tabs {
    fn as_ref(&self) -> Option<&Document> {
        self.docs.get(self.active)
    }

    fn as_mut(&mut self) -> Option<&mut Document> {
        self.docs.get_mut(self.active)
    }

    /// Adds a document and makes it the shown one.
    fn push(&mut self, doc: Document) {
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
    }

    /// Closes the shown document. Returns whether any remain.
    fn close_active(&mut self) -> bool {
        if self.docs.is_empty() {
            return false;
        }
        self.docs.remove(self.active);
        if self.active >= self.docs.len() && !self.docs.is_empty() {
            self.active = self.docs.len() - 1;
        }
        !self.docs.is_empty()
    }

    fn cycle(&mut self, forward: bool) {
        let n = self.docs.len();
        if n < 2 {
            return;
        }
        self.active = if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        };
    }

    fn len(&self) -> usize {
        self.docs.len()
    }

    /// The tab strip's labels — each document's file name, with §8.1's dot when it grew unseen —
    /// and which is active, for drawing. Asking clears the shown tab's dot: it is being seen.
    fn labels(&mut self) -> Vec<String> {
        let active = self.active;
        if let Some(doc) = self.docs.get_mut(active) {
            doc.unseen = false;
        }
        self.docs
            .iter()
            .map(|d| {
                if d.unseen {
                    format!("● {}", d.summary)
                } else {
                    d.summary.clone()
                }
            })
            .collect()
    }
}

/// A watched folder — §8.1: "a directory plus a glob; new matching files are adopted as they
/// appear." The glob is the simple kind a shell offers: `*` and `?` in a file name.
struct Watch {
    dir: std::path::PathBuf,
    /// A file-name pattern such as `*.log`; `*` alone matches everything.
    pattern: String,
    /// Files already adopted (or seen at start), so a scan opens only what is new.
    known: std::collections::HashSet<std::path::PathBuf>,
}

/// Follow ticks between folder scans: 2 s at 100 ms — a directory listing is not a length check.
const WATCH_EVERY_TICKS: u64 = 20;

impl Watch {
    /// From a command-line argument: a directory (glob `*.log`), or `dir\*.txt`-style. `None` if
    /// the argument is neither.
    fn from_arg(arg: &std::path::Path) -> Option<Self> {
        if arg.is_dir() {
            return Some(Self {
                dir: arg.to_path_buf(),
                pattern: "*.log".to_owned(),
                known: std::collections::HashSet::new(),
            });
        }
        let name = arg.file_name()?.to_string_lossy().into_owned();
        if !name.contains(['*', '?']) {
            return None;
        }
        let dir = arg.parent().filter(|p| !p.as_os_str().is_empty())?;
        dir.is_dir().then(|| Self {
            dir: dir.to_path_buf(),
            pattern: name,
            known: std::collections::HashSet::new(),
        })
    }

    fn matches(pattern: &str, name: &str) -> bool {
        fn go(p: &[char], n: &[char]) -> bool {
            match (p.first(), n.first()) {
                (None, None) => true,
                (Some('*'), _) => go(&p[1..], n) || (!n.is_empty() && go(p, &n[1..])),
                (Some('?'), Some(_)) => go(&p[1..], &n[1..]),
                (Some(a), Some(b)) if a.eq_ignore_ascii_case(b) => go(&p[1..], &n[1..]),
                _ => false,
            }
        }
        let p: Vec<char> = pattern.chars().collect();
        let n: Vec<char> = name.chars().collect();
        go(&p, &n)
    }

    /// The matching files not yet known, oldest first by modification time, and marks them known.
    fn new_files(&mut self) -> Vec<std::path::PathBuf> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut fresh: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let name = path.file_name()?.to_string_lossy().into_owned();
                if !path.is_file() || !Self::matches(&self.pattern, &name) {
                    return None;
                }
                if self.known.contains(&path) {
                    return None;
                }
                let modified = e.metadata().and_then(|m| m.modified()).ok()?;
                Some((modified, path))
            })
            .collect();
        fresh.sort();
        for (_, path) in &fresh {
            self.known.insert(path.clone());
        }
        fresh.into_iter().map(|(_, p)| p).collect()
    }
}

/// The bar's colours. Provisional with the rest of the palette; a shade off the ground so the bar
/// reads as chrome, the focused field a shade lighter than the other.
const CHROME_BG: [f32; 4] = [0.10, 0.11, 0.13, 1.0];
const FIELD_BG: [f32; 4] = [0.14, 0.15, 0.18, 1.0];
const FIELD_BG_FOCUSED: [f32; 4] = [0.18, 0.20, 0.24, 1.0];
const FIELD_HINT: [f32; 4] = [0.42, 0.45, 0.50, 1.0];
const FIELD_SELECTION_BG: [f32; 4] = [0.20, 0.36, 0.60, 1.0];
const CARET: [f32; 4] = [0.88, 0.89, 0.91, 1.0];
const CHIP_INCLUDE_BG: [f32; 4] = [0.14, 0.26, 0.20, 1.0];
const CHIP_EXCLUDE_BG: [f32; 4] = [0.30, 0.16, 0.16, 1.0];
const TAB_BG: [f32; 4] = [0.13, 0.14, 0.17, 1.0];
const TAB_ACTIVE_BG: [f32; 4] = [0.20, 0.22, 0.26, 1.0];

/// One field: its text (cut from the left to fit), a hint when empty and unfocused, the selection
/// as a background span, the caret as a two-pixel fill, and a mark under an IME composition.
#[allow(clippy::too_many_arguments)]
fn draw_field(
    painter: &mut Painter,
    view: &View,
    cells: &CellModel,
    field: &TextField,
    focused: bool,
    x: f32,
    y: f32,
    width_cells: usize,
    hint: &str,
) {
    let cell_w = painter.cell_width();
    let row_h = painter.row_height();
    let display = field.display();
    if display.is_empty() && !focused {
        let _ = painter.lay_out_at(view, x, y, hint, Colours::plain(FIELD_HINT));
        return;
    }
    let (shown, cut) = Chrome::fitted(cells, &display, width_cells);
    // The selection, as a span with a background — the painter's own way of filling behind text.
    let mut spans = Vec::new();
    if let Some(sel) = field.selection() {
        let start = sel.start.saturating_sub(cut);
        let end = sel.end.saturating_sub(cut);
        if start < end && end <= shown.len() {
            spans.push(Span {
                start,
                end,
                fg: None,
                bg: Some(FIELD_SELECTION_BG),
            });
        }
    }
    let _ = painter.lay_out_at(
        view,
        x,
        y,
        shown,
        Colours {
            tint: INK,
            selected: None,
            spans: &spans,
        },
    );
    if !focused {
        return;
    }
    // The composition's mark: a thin line under it.
    if let Some(comp) = field.display_composition() {
        let from = cells.cell_at_byte(shown, comp.start.saturating_sub(cut).min(shown.len()));
        let to = cells.cell_at_byte(shown, comp.end.saturating_sub(cut).min(shown.len()));
        painter.fill(
            x + from as f32 * cell_w,
            y + row_h - 2.0,
            (to.saturating_sub(from)) as f32 * cell_w,
            1.0,
            CARET,
        );
    }
    let caret_byte = field.display_caret().saturating_sub(cut).min(shown.len());
    let caret_cell = cells.cell_at_byte(shown, caret_byte);
    painter.fill(x + caret_cell as f32 * cell_w, y, 2.0, row_h, CARET);
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
    /// The measured cell size, for the command bar's hit-test. Zero until the first frame.
    cell_w: f32,
    cell_h: f32,
    /// §12.4's persisted state, its tiers, and whether writes are suppressed. Loaded at start;
    /// written on close and when a tab closes.
    settings: settings::Settings,
    settings_tiers: Vec<std::path::PathBuf>,
    stateless: bool,
    /// `None` until the worker hands the device over. While it is `None` the class background
    /// brush is doing the painting — stage one of the two-stage paint.
    renderer: Option<Renderer>,
    pending: Option<Receiver<std::result::Result<Renderer, tailhawk_core::Error>>>,
    /// What the two workers have reported so far. Either can land first, so the title is rebuilt
    /// from both rather than written by whichever finishes.
    driver: Option<String>,
    /// Files being opened on workers — one receiver each. Several at once is a file set from the
    /// command line or a watched folder adopting new files (§8.1).
    reading: Vec<Receiver<std::result::Result<Document, String>>>,
    /// Watched folders — a directory and a glob; new matching files are adopted as tabs as they
    /// appear (§8.1). Scanned on the follow tick, every [`WATCH_EVERY_TICKS`].
    watching: Vec<Watch>,
    ticks: u64,
    file: Option<String>,
    document: Tabs,
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
        let mut landed = false;
        let mut i = 0;
        while i < self.reading.len() {
            match self.reading[i].try_recv() {
                Ok(Ok(mut document)) => {
                    self.reading.remove(i);
                    // §12.4: the file's remembered view, before it is first drawn.
                    let key = document
                        .path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned());
                    let remembered = key.and_then(|k| self.settings.file(&k).cloned());
                    if let Some(state) = remembered {
                        document.apply_state(&state);
                    }
                    self.file = Some(document.describe());
                    self.document.push(document);
                    landed = true;
                }
                Ok(Err(e)) => {
                    self.reading.remove(i);
                    self.file = Some(e);
                    landed = true;
                }
                Err(TryRecvError::Disconnected) => {
                    self.reading.remove(i);
                    self.file = Some("read failed".to_owned());
                    landed = true;
                }
                Err(TryRecvError::Empty) => i += 1,
            }
        }
        if !landed {
            return;
        }
        self.refresh_title(hwnd);
        // Tailing starts the moment there is something to tail; and the file only becomes visible
        // on the next frame, which nothing else will ask for — the window is otherwise idle.
        unsafe {
            SetTimer(hwnd, FOLLOW_TIMER, FOLLOW_POLL_MS, None);
            let _ = InvalidateRect(hwnd, None, false);
        }
    }

    /// Writes §12.4's state: where the window is, and how each open file is being looked at.
    fn save_settings(&mut self, hwnd: HWND) {
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_ok() {
            let r = placement.rcNormalPosition;
            self.settings.window = Some(settings::Window {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
                maximized: placement.showCmd == SW_SHOWMAXIMIZED.0 as u32,
            });
        }
        for doc in &self.document.docs {
            if let Some(state) = doc.file_state() {
                self.settings.set_file(state);
            }
        }
        settings::save(&self.settings_tiers, &self.settings, self.stateless);
    }

    /// The status: the driver, the document's description, the frame instrument. **In the title,
    /// where a measurement rig can read it, and in the status bar, where a user does.**
    fn status_text(&self) -> String {
        let mut text = String::new();
        for part in [self.driver.as_deref(), self.file.as_deref()]
            .into_iter()
            .flatten()
        {
            if !text.is_empty() {
                text.push_str(" — ");
            }
            text.push_str(part);
        }
        // **The frame instrument, where a user and a measurement rig can both see it.** M4 asks for
        // "without dropped frames" and nothing in the product could say whether that held; the
        // throughput rig could only measure how long the window took to answer a message, which
        // counts a vsync-blocked Present the same as a seized thread.
        if let Some((p95, worst, over)) = self.frames.summary() {
            text.push_str(&format!(
                " — frame p95 {p95:.1} ms, worst {worst:.1} ms, {over} over budget"
            ));
        }
        text
    }

    fn refresh_title(&self, hwnd: HWND) {
        let status = self.status_text();
        let title = if status.is_empty() {
            String::from("Tailhawk")
        } else {
            format!("Tailhawk — {status}")
        };
        set_title(hwnd, &title);
        if self.pending.is_none() && self.reading.is_empty() {
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
        // The strip and the status are the shell's knowledge, handed to the document that draws them.
        let strip = (self.document.labels(), self.document.active);
        let status = self.status_text();
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
                    self.cell_w = cell.0;
                    self.cell_h = cell.1;
                    doc.tab_strip = strip;
                    doc.status = status;
                    doc.lay_out(cell, (w, h));
                    // The highlighter's frame budget starts here, alongside the painter's own
                    // `begin_frame` inside `paint_rows` — one frame, one budget, §11.3.
                    doc.highlighter.begin_frame();
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
        // The `SetWindowPos` that precedes this sends a `WM_SIZE` **nested** inside the
        // `WM_DPICHANGED` handler, and `wndproc` drops nested messages (see its note). So the
        // swap chain is resized here, from the client size the window now has.
        self.resize(hwnd);
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
        set_clipboard(&text)
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

    /// Rebuilds the title from the document's live state.
    ///
    /// **The cached string is refreshed from the document rather than edited**, because every part
    /// of it except the file name can change while the window is open — the counts, the membership,
    /// the stream state and now the find state. `Document::describe` is the one place that knows the
    /// current answer, and a title assembled from remembered fragments is how "2 files … newest is
    /// log_002.txt" survived onto a window showing three.
    fn retitle(&mut self, hwnd: HWND) {
        if let Some(doc) = self.document.as_ref() {
            self.file = Some(doc.describe());
        }
        self.refresh_title(hwnd);
    }

    /// View toggles and the open command.
    ///
    /// `Ctrl+O` is §12's open file — brought forward from M7's shell because a file that can only
    /// be opened from a command line is not a tool the owner can reach for. `Ctrl+I` is §13.4's
    /// **reveal invisibles** — a key of our choosing, because §12 gives the toggle no binding and
    /// the command palette that would carry it is M7. Recorded as provisional in `CLEANROOM.md`.
    fn view_key(&mut self, hwnd: HWND, key: u16, ctrl: bool) -> bool {
        if !ctrl {
            return false;
        }
        // §12: `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle the tabs, `Ctrl+W` closes the shown one. A last
        // tab closed leaves the window open and empty, as a browser would not but a viewer should:
        // the next `Ctrl+O` or drop fills it.
        if key == VK_TAB.0 || key == VK_W.0 {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            if key == VK_TAB.0 {
                self.document.cycle(!shift);
            } else {
                self.document.close_active();
                if self.document.len() == 0 {
                    self.file = None;
                }
            }
            self.retitle(hwnd);
            self.sync_scrollbar(hwnd);
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
            return true;
        }
        // `Ctrl+O` is not here: the dialog pumps messages and `wndproc` would re-enter `STATE`
        // while this borrow is held. It is dispatched before the borrow, in `wndproc`.
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if key == VK_E.0 {
            // §6.4's collapse, as a toggle: only with a format, since only a format knows what a
            // first line is. Provisional binding, like `Ctrl+I`.
            if doc.detection.accepted.is_none() {
                return true;
            }
            doc.filtering.records_only = !doc.filtering.records_only;
            doc.filtering.clear_results();
            doc.refilter();
            {
                let rows = doc.view_rows();
                doc.view.grid_mut().set_total_rows(rows);
            }
            self.sync_scrollbar(hwnd);
            self.retitle(hwnd);
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
            return true;
        }
        if key != VK_I.0 {
            return false;
        }
        let cells = doc.view.cells_mut();
        cells.reveal_invisibles = !cells.reveal_invisibles;
        self.retitle(hwnd);
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
        true
    }

    /// Opens `path` in this window, replacing what is shown. The read runs on a worker as it does
    /// at start-up, and the title says "opening" until it lands — a large file takes seconds to
    /// index and a window that went blank without a word would look hung.
    fn open_path(&mut self, hwnd: HWND, path: std::path::PathBuf) {
        self.file = Some(format!("opening {}…", path.display()));
        self.reading.push(spawn_open(move || Document::open(&path)));
        self.refresh_title(hwnd);
        unsafe {
            SetTimer(hwnd, DEVICE_POLL_TIMER, DEVICE_POLL_MS, None);
            let _ = InvalidateRect(hwnd, None, false);
        }
    }

    /// A file dropped on the window: the first of them is opened. §12's drop target — brought
    /// forward with `Ctrl+O`, for the same reason.
    fn dropped(&mut self, hwnd: HWND, drop: HDROP) {
        let mut buf = [0u16; 32_768];
        let len = unsafe { DragQueryFileW(drop, 0, Some(&mut buf)) } as usize;
        unsafe { DragFinish(drop) };
        if len == 0 {
            return;
        }
        let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len]));
        self.open_path(hwnd, path);
    }

    /// One keystroke, offered to the command bar before anything else.
    ///
    /// `Ctrl+F` focuses the find field with its text selected, `Ctrl+L` / `Ctrl+Shift+L` the new-chip
    /// field for an include / an exclude — `UI-DESIGN.md` §12. While a field has focus the editing
    /// keys are its: caret moves (`Ctrl` by word, `Shift` extends), `Home`/`End`, `Backspace`,
    /// `Delete`, `Ctrl+A/Z/Y/X/C/V`, `Enter` to act, `Esc` to hand the keyboard back to the grid.
    /// **Everything else falls through** — `PageDown` still pages the grid with a query half-typed,
    /// which is what a field beside a grid should allow — and `F3` steps whether or not the field
    /// has focus.
    fn chrome_key(&mut self, hwnd: HWND, key: u16, ctrl: bool, shift: bool) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let handled = if ctrl && key == VK_F.0 {
            doc.chrome.focus = Focus::Find;
            doc.chrome.find.select_all();
            doc.finder.error = None;
            true
        } else if ctrl && key == VK_L.0 {
            doc.chrome.focus = Focus::NewChip;
            doc.chrome.chip_polarity = if shift {
                Polarity::Exclude
            } else {
                Polarity::Include
            };
            doc.filtering.error = None;
            true
        } else {
            match doc.chrome.focus {
                Focus::Grid => false,
                focus => {
                    let Some(field) = doc.chrome.focused() else {
                        return false;
                    };
                    match key {
                        k if k == VK_LEFT.0 => {
                            field.move_caret(if ctrl { Move::WordLeft } else { Move::Left }, shift)
                        }
                        k if k == VK_RIGHT.0 => field
                            .move_caret(if ctrl { Move::WordRight } else { Move::Right }, shift),
                        k if k == VK_HOME.0 => field.move_caret(Move::Home, shift),
                        k if k == VK_END.0 => field.move_caret(Move::End, shift),
                        k if k == VK_BACK.0 => field.backspace(),
                        k if k == VK_DELETE.0 => field.delete(),
                        k if ctrl && k == VK_A.0 => field.select_all(),
                        k if ctrl && k == VK_Z.0 => {
                            field.undo();
                        }
                        k if ctrl && k == VK_Y.0 => {
                            field.redo();
                        }
                        k if ctrl && k == VK_X.0 => {
                            if let Some(cut) = field.cut() {
                                set_clipboard(&cut);
                            }
                        }
                        k if ctrl && k == VK_C.0 => {
                            if let Some(sel) = field.selected_text() {
                                set_clipboard(sel);
                            }
                        }
                        k if ctrl && k == VK_V.0 => {
                            if let Some(text) = clipboard_text() {
                                field.paste(&text);
                            }
                        }
                        k if k == VK_ESCAPE.0 => doc.chrome.focus = Focus::Grid,
                        k if k == VK_RETURN.0 => match focus {
                            Focus::Find => {
                                doc.finder.query = doc.chrome.find.text().to_owned();
                                doc.find();
                            }
                            Focus::NewChip => {
                                let text = doc.chrome.chip.text().to_owned();
                                let polarity = doc.chrome.chip_polarity;
                                doc.add_chip(&text, polarity);
                                doc.chrome.chip.set_text("");
                            }
                            Focus::Grid => {}
                        },
                        _ => return false,
                    }
                    true
                }
            }
        };
        if !handled {
            return false;
        }
        self.sync_scrollbar(hwnd);
        self.retitle(hwnd);
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
        true
    }

    /// The keys that act on results rather than fields, in whichever focus: `F3` / `Shift+F3` step
    /// the search; `Esc` with the grid focused unwinds — the finder's results first, then the chips.
    fn find_key(&mut self, hwnd: HWND, key: u16, _ctrl: bool, shift: bool) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        let moved = if key == VK_F3.0 {
            // A step with no results but a query is the search the user meant — pressing `F3` after
            // `Esc` should look for the thing, not do nothing.
            if doc.finder.matches.is_empty() && doc.finder.running.is_none() {
                doc.finder.query = doc.chrome.find.text().to_owned();
                doc.find();
                false
            } else {
                doc.find_step(!shift)
            }
        } else if key == VK_ESCAPE.0 && doc.chrome.focus == Focus::Grid {
            if !doc.finder.matches.is_empty() || doc.finder.running.is_some() {
                doc.finder.clear();
            } else if doc.filtering.active() {
                doc.clear_filter();
            } else {
                return false;
            }
            false
        } else {
            return false;
        };
        if moved {
            self.sync_scrollbar(hwnd);
        }
        self.retitle(hwnd);
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
        true
    }

    /// One typed character, into whichever field has focus. **Surrogate pairs are joined here**:
    /// `WM_CHAR` delivers a non-BMP character as two messages, and inserting each on its own puts
    /// two replacement characters into the field.
    fn find_char(&mut self, hwnd: HWND, unit: u16) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if doc.chrome.focus == Focus::Grid {
            return false;
        }
        let mut text = String::new();
        if !push_typed_unit(&mut text, &mut doc.chrome.pending_high, unit) {
            return false;
        }
        if let Some(field) = doc.chrome.focused() {
            field.insert(&text);
        }
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
        true
    }

    /// An IME message for the focused field. `WM_IME_COMPOSITION` carries the in-progress string
    /// (`GCS_COMPSTR`, with its cursor at `GCS_CURSORPOS`) or the settled one (`GCS_RESULTSTR`);
    /// the field shows the first in place and commits the second. Returns whether it was consumed
    /// — when it is, `DefWindowProcW` must not see the message, or it would turn the result into
    /// `WM_CHAR`s and the text would arrive twice. Also parks the IME's candidate window at the
    /// caret, so it opens beside what is being typed rather than at the window's corner.
    fn ime(&mut self, hwnd: HWND, msg: u32, lparam: LPARAM) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if doc.chrome.focus == Focus::Grid {
            return false;
        }
        let (find_origin, chip_origin) = doc.chrome.origins.get();
        let origin = if doc.chrome.focus == Focus::Find {
            find_origin
        } else {
            chip_origin
        };
        let cell_w = self.cell_w.max(1.0);
        let cells = *doc.view.cells();
        let Some(field) = doc.chrome.focused() else {
            return false;
        };
        let himc = unsafe { ImmGetContext(hwnd) };
        if himc.is_invalid() {
            return false;
        }
        let read =
            |what: windows::Win32::UI::Input::Ime::IME_COMPOSITION_STRING| -> Option<String> {
                let bytes = unsafe { ImmGetCompositionStringW(himc, what, None, 0) };
                if bytes < 0 {
                    return None;
                }
                let mut buf = vec![0u16; (bytes as usize) / 2];
                if !buf.is_empty() {
                    unsafe {
                        ImmGetCompositionStringW(
                            himc,
                            what,
                            Some(buf.as_mut_ptr().cast()),
                            bytes as u32,
                        )
                    };
                }
                Some(String::from_utf16_lossy(&buf))
            };
        let flags = lparam.0 as u32;
        let mut consumed = false;
        match msg {
            WM_IME_STARTCOMPOSITION => {
                field.set_composition("", 0);
                consumed = true;
            }
            WM_IME_COMPOSITION => {
                if flags & GCS_RESULTSTR.0 != 0 {
                    if let Some(result) = read(GCS_RESULTSTR) {
                        field.commit_composition(&result);
                        consumed = true;
                    }
                }
                if flags & GCS_COMPSTR.0 != 0 {
                    if let Some(comp) = read(GCS_COMPSTR) {
                        let cursor =
                            unsafe { ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0) };
                        let cursor_utf16 = cursor.max(0) as usize;
                        let caret = comp
                            .encode_utf16()
                            .take(cursor_utf16)
                            .collect::<Vec<u16>>()
                            .len();
                        let caret_bytes = String::from_utf16_lossy(
                            &comp.encode_utf16().take(caret).collect::<Vec<_>>(),
                        )
                        .len();
                        field.set_composition(&comp, caret_bytes);
                        consumed = true;
                    }
                }
            }
            WM_IME_ENDCOMPOSITION => {
                field.clear_composition();
                consumed = true;
            }
            _ => {}
        }
        // The candidate window, at the caret.
        let display = field.display();
        let (shown, cut) = Chrome::fitted(&cells, &display, FIND_CELLS.max(CHIP_CELLS));
        let caret_cell = cells.cell_at_byte(
            shown,
            field.display_caret().saturating_sub(cut).min(shown.len()),
        );
        let form = COMPOSITIONFORM {
            dwStyle: CFS_POINT,
            ptCurrentPos: windows::Win32::Foundation::POINT {
                x: (origin + caret_cell as f32 * cell_w) as i32,
                y: doc.view.chrome_px() as i32,
            },
            ..Default::default()
        };
        unsafe {
            let _ = ImmSetCompositionWindow(himc, &form);
            let _ = ImmReleaseContext(hwnd, himc);
            let _ = InvalidateRect(hwnd, None, false);
        }
        consumed
    }

    /// A click in the command bar: a field takes focus and the caret lands where the click was; a
    /// chip is removed. Returns whether the click was the bar's.
    fn chrome_click(&mut self, hwnd: HWND, x: f32, y: f32, extend: bool) -> bool {
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        if y >= doc.view.chrome_px() {
            if doc.chrome.focus != Focus::Grid {
                doc.chrome.focus = Focus::Grid;
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            return false;
        }
        // The strip is the top band; a click there is a tab, and only a tab.
        let in_strip = doc.tab_strip.0.len() > 1 && y < Chrome::strip_height(self.cell_h.max(1.0));
        let hit = doc
            .chrome
            .hits
            .borrow()
            .iter()
            .find(|(range, hit)| range.contains(&x) && matches!(hit, Hit::Tab(_)) == in_strip)
            .map(|(_, hit)| *hit);
        let (find_origin, chip_origin) = doc.chrome.origins.get();
        let cell_w = self.cell_w.max(1.0);
        if let Some(Hit::Tab(i)) = hit {
            self.document.active = i.min(self.document.len().saturating_sub(1));
            self.retitle(hwnd);
            self.sync_scrollbar(hwnd);
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
            return true;
        }
        let Some(doc) = self.document.as_mut() else {
            return false;
        };
        match hit {
            Some(Hit::Tab(_)) => {}
            Some(Hit::Chip(i)) => {
                if let Some(chip) = doc.filtering.chips.chips.get_mut(i) {
                    chip.enabled = !chip.enabled;
                    doc.filtering.clear_results();
                    doc.refilter();
                    {
                        let rows = doc.view_rows();
                        doc.view.grid_mut().set_total_rows(rows);
                    }
                }
            }
            Some(Hit::ChipClose(i)) => {
                if i < doc.filtering.chips.chips.len() {
                    doc.filtering.chips.chips.remove(i);
                    doc.filtering.clear_results();
                    doc.refilter();
                    {
                        let rows = doc.view_rows();
                        doc.view.grid_mut().set_total_rows(rows);
                    }
                }
            }
            Some(hit @ (Hit::Find | Hit::NewChip)) => {
                let (origin, focus) = match hit {
                    Hit::Find => (find_origin, Focus::Find),
                    _ => (chip_origin, Focus::NewChip),
                };
                doc.chrome.focus = focus;
                let width = if hit == Hit::Find {
                    FIND_CELLS
                } else {
                    CHIP_CELLS
                };
                let cells = *doc.view.cells();
                if let Some(field) = doc.chrome.focused() {
                    let display = field.display();
                    let (shown, cut) = Chrome::fitted(&cells, &display, width);
                    let cell = ((x - origin) / cell_w).max(0.0).round() as usize;
                    let byte = cells.byte_at_cell(shown, cell) + cut;
                    field.place(byte, extend);
                }
            }
            None => doc.chrome.focus = Focus::Grid,
        }
        self.sync_scrollbar(hwnd);
        self.retitle(hwnd);
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
        true
    }

    /// Collects what the filter worker has reported, and repaints if the view changed.
    fn poll_filter(&mut self, hwnd: HWND) {
        let changed = self.document.as_mut().is_some_and(Document::poll_filter);
        if !changed {
            return;
        }
        self.sync_scrollbar(hwnd);
        self.retitle(hwnd);
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
        }
    }

    /// Collects what the search worker has reported, and repaints if it said anything.
    fn poll_find(&mut self, hwnd: HWND) {
        let changed = self.document.as_mut().is_some_and(Document::poll_find);
        if !changed {
            return;
        }
        self.sync_scrollbar(hwnd);
        self.retitle(hwnd);
        unsafe {
            let _ = InvalidateRect(hwnd, None, false);
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

/// The system open dialog. Modal on the window thread, which is what it is; the follow tick and
/// the paint keep running underneath it because it pumps our messages too.
fn ask_for_file(hwnd: HWND) -> Option<std::path::PathBuf> {
    let mut file = [0u16; 32_768];
    let filter: Vec<u16> = "Log files (*.log;*.txt)\0*.log;*.txt\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
        nMaxFile: file.len() as u32,
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY,
        ..Default::default()
    };
    if !unsafe { GetOpenFileNameW(&mut ofn) }.as_bool() {
        return None;
    }
    let len = file.iter().position(|&c| c == 0).unwrap_or(file.len());
    Some(std::path::PathBuf::from(String::from_utf16_lossy(
        &file[..len],
    )))
}

/// Puts `text` on the clipboard as `CF_UNICODETEXT`.
///
/// `GlobalFree` is on the failure path only — freeing after a successful hand-over is a double
/// free of memory the system now owns. `CloseClipboard` runs on every path, including the ones
/// that fail, because leaving it open locks every other application out of the clipboard.
fn set_clipboard(text: &str) -> bool {
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

/// The clipboard's text, if it holds any — for a field's paste. Line breaks are folded to spaces:
/// a one-line field has nowhere to put them.
fn clipboard_text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let text = GetClipboardData(CF_UNICODETEXT.0 as u32)
            .ok()
            .and_then(|handle| {
                let global = windows::Win32::Foundation::HGLOBAL(handle.0);
                let ptr = GlobalLock(global).cast::<u16>();
                if ptr.is_null() {
                    return None;
                }
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                let _ = GlobalUnlock(global);
                Some(text)
            });
        let _ = CloseClipboard();
        text.map(|t| t.replace(['\r', '\n'], " "))
    }
}

fn stop_polling(hwnd: HWND) {
    unsafe {
        let _ = KillTimer(hwnd, DEVICE_POLL_TIMER);
    }
}

thread_local! {
    /// Whether `wndproc` is already on the stack. See [`wndproc`].
    static IN_WNDPROC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The window procedure, guarded against **re-entry**.
///
/// Every handler below borrows `STATE` mutably for its duration. Windows re-enters this function
/// synchronously whenever a handler makes a call that pumps or sends messages — a modal dialog
/// (`GetOpenFileNameW`), `SetWindowTextW`, `SetWindowPos`, `SetScrollInfo` — and a re-entered
/// handler that borrows again is a `RefCell` panic and an aborted process. **That happened**, on
/// the first wiring of `Ctrl+O`, and only a run of the binary found it. So the rule is enforced
/// here rather than remembered per handler: a message that arrives while one is being handled goes
/// straight to `DefWindowProcW`. Nothing this window handles needs to be handled *nested* — a
/// nested `WM_PAINT` under a dialog is the class brush for a moment, a nested `WM_TIMER` is the
/// next tick's — and anything that did would today be the crash this prevents.
extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if IN_WNDPROC.with(|flag| flag.replace(true)) {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    let result = handle(hwnd, msg, wparam, lparam);
    IN_WNDPROC.with(|flag| flag.set(false));
    result
}

fn handle(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION => {
            let consumed = STATE.with(|s| {
                s.borrow_mut()
                    .as_mut()
                    .is_some_and(|shell| shell.ime(hwnd, msg, lparam))
            });
            if consumed {
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
        WM_DROPFILES => {
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.dropped(hwnd, HDROP(wparam.0 as *mut core::ffi::c_void));
                }
            });
            LRESULT(0)
        }
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
            // Every tab follows, so switching to one is not a jump; only the shown one repaints.
            // A tab that grew while not shown gets §8.1's dot. Watched folders are scanned every
            // WATCH_EVERY_TICKS and adopt what is new as tabs.
            let grew = STATE.with(|s| {
                let mut state = s.borrow_mut();
                let Some(shell) = state.as_mut() else {
                    return false;
                };
                let active = shell.document.active;
                let mut shown_grew = false;
                for (i, doc) in shell.document.docs.iter_mut().enumerate() {
                    if doc.poll_follow() {
                        if i == active {
                            shown_grew = true;
                        } else {
                            doc.unseen = true;
                        }
                    }
                }
                shell.ticks += 1;
                if shell.ticks % WATCH_EVERY_TICKS == 0 {
                    let fresh: Vec<std::path::PathBuf> = shell
                        .watching
                        .iter_mut()
                        .flat_map(Watch::new_files)
                        .collect();
                    for path in fresh {
                        shell
                            .reading
                            .push(spawn_open(move || Document::open(&path)));
                    }
                    if !shell.reading.is_empty() {
                        unsafe {
                            SetTimer(hwnd, DEVICE_POLL_TIMER, DEVICE_POLL_MS, None);
                        }
                    }
                }
                shown_grew
            });
            if grew {
                // The counts moved, so the title is now wrong until it is rebuilt.
                STATE.with(|s| {
                    let mut state = s.borrow_mut();
                    if let Some(shell) = state.as_mut() {
                        shell.retitle(hwnd);
                        shell.sync_scrollbar(hwnd);
                    }
                });
                unsafe {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            // **On the same tick as following, deliberately.** A search is the other thing that
            // arrives from a worker while the window is idle, and §7.4 wants its results streamed
            // rather than waited for; a second timer would buy latency the eye cannot see and add
            // a second thing to stop when the document closes.
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.poll_find(hwnd);
                    shell.poll_filter(hwnd);
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

            // **`Ctrl+O` before anything borrows `STATE`.** `GetOpenFileNameW` is modal and pumps
            // this window's messages, so `wndproc` re-enters while it is up; a borrow held across
            // it is a `RefCell` panic and an aborted process — which is exactly what the first
            // wiring did.
            if ctrl && wparam.0 as u16 == VK_O.0 {
                if let Some(path) = ask_for_file(hwnd) {
                    STATE.with(|s| {
                        if let Some(shell) = s.borrow_mut().as_mut() {
                            shell.open_path(hwnd, path);
                        }
                    });
                }
                return LRESULT(0);
            }

            // **The find state is offered the key first**, because `Esc`, `Enter` and `Backspace`
            // mean something to it and nothing to the navigation map — and because `F3` must not
            // fall through to `DefWindowProcW` once there is something for it to do.
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let consumed = STATE.with(|s| {
                s.borrow_mut().as_mut().is_some_and(|shell| {
                    shell.view_key(hwnd, wparam.0 as u16, ctrl)
                        || shell.chrome_key(hwnd, wparam.0 as u16, ctrl, shift)
                        || shell.find_key(hwnd, wparam.0 as u16, ctrl, shift)
                })
            });
            if consumed {
                return LRESULT(0);
            }

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
        // Typed text, which only exists as a message because `TranslateMessage` produces it from
        // the key pair. It carries the keyboard layout, the dead keys and the IME's output — which
        // is why a query is built from `WM_CHAR` and not from virtual-key codes.
        WM_CHAR => {
            let consumed = STATE.with(|s| {
                s.borrow_mut()
                    .as_mut()
                    .is_some_and(|shell| shell.find_char(hwnd, wparam.0 as u16))
            });
            if consumed {
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
                // The command bar first: a click in it is a field taking focus or a chip going,
                // never a selection.
                if msg == WM_LBUTTONDOWN && shell.chrome_click(hwnd, x, y, shift) {
                    return;
                }
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
        WM_CLOSE => {
            // §12.4: the window and every open file's view, written whole to the first writable
            // tier — the last thing this window does. **Here and not in `WM_DESTROY`**: the default
            // `WM_CLOSE` handling calls `DestroyWindow`, whose `WM_DESTROY` arrives *nested* and
            // so goes to `DefWindowProcW` under the re-entry guard — a save there never ran, and
            // neither did the quit. Both are done here, before the window goes.
            STATE.with(|s| {
                if let Some(shell) = s.borrow_mut().as_mut() {
                    shell.save_settings(hwnd);
                }
            });
            unsafe {
                let _ = DestroyWindow(hwnd);
                PostQuitMessage(0);
            }
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

    // Every positional argument opens — a file set from the command line, §8.1 — and a directory
    // or a `dir\*.log` glob is a watched folder: its matching files open now and new ones are
    // adopted as they appear. `-` is the standard input stream by convention.
    let mut reading = Vec::new();
    let mut watching = Vec::new();
    let mut any_arg = false;
    for arg in std::env::args_os().skip(1) {
        any_arg = true;
        if arg == "-" {
            reading.push(spawn_open(Document::from_pipe));
            continue;
        }
        let path = std::path::PathBuf::from(&arg);
        if let Some(mut watch) = Watch::from_arg(&path) {
            for file in watch.new_files() {
                reading.push(spawn_open(move || Document::open(&file)));
            }
            watching.push(watch);
            continue;
        }
        reading.push(spawn_open(move || Document::open(&path)));
    }
    // **No path, so look at the standard input handle** — §4.2. `FILE_TYPE_CHAR` is an
    // interactive console and §4.2 says "do not block": reading it would wait for a human to
    // type, which for a windowed application means a window that never appears.
    if !any_arg && stdin_kind().readable() {
        reading.push(spawn_open(Document::from_pipe));
    }

    // §12.4: the settings tiers — exe-adjacent, then %APPDATA%Tailhawk — read and merged now,
    // written on close. `--stateless` suppresses the writes and nothing else.
    let stateless = std::env::args_os().any(|a| a == "--stateless");
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
    let roaming = std::env::var_os("APPDATA").map(std::path::PathBuf::from);
    let settings_tiers = settings::tiers(exe_dir.as_deref(), roaming.as_deref());
    let settings = settings::load(&settings_tiers);
    let placement = settings.window;

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
            cell_w: 0.0,
            cell_h: 0.0,
            settings,
            settings_tiers,
            stateless,
            renderer: None,
            pending: Some(rx),
            driver: None,
            reading,
            watching,
            ticks: 0,
            document: Tabs::default(),

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
            placement.map_or(CW_USEDEFAULT, |w| w.x),
            placement.map_or(CW_USEDEFAULT, |w| w.y),
            placement.map_or(1280, |w| w.width.max(320)),
            placement.map_or(800, |w| w.height.max(200)),
            None,
            None,
            instance,
            None,
        )?
    };
    unsafe {
        // §12's drop target: a file dropped on the window opens in it.
        DragAcceptFiles(hwnd, true);
        SetTimer(hwnd, DEVICE_POLL_TIMER, DEVICE_POLL_MS, None);
        let _ = ShowWindow(
            hwnd,
            if placement.is_some_and(|w| w.maximized) {
                SW_SHOWMAXIMIZED
            } else {
                SW_SHOW
            },
        );
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
        // 20 rows of 10 px in a 200 px grid — plus the command bar's band above and the status
        // bar's below.
        doc.lay_out(
            (8.0, 10.0),
            (
                800,
                200 + Chrome::height(10.0) as u32 + Chrome::strip_height(10.0) as u32,
            ),
        );
        assert_eq!(doc.set.total_rows(), 5_000);
        // A tail tool opens at the tail, following.
        assert_eq!(doc.view.grid().scroll().row, 4_980);
        assert!(doc.view.grid().is_following());
        assert!(doc.navigate(Navigate::DocStart));
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

    fn hit(line: u64, start: usize) -> Match {
        Match {
            line,
            start,
            end: start + 5,
        }
    }

    /// **The ordered match list is maintained, never rebuilt** — see [`Finder::absorb`].
    ///
    /// §7.4 reports chunks out of order, so this feeds them in the worst order it can and asserts
    /// what a sort would have produced. The second assertion is the one with teeth: an insertion
    /// *before* the current match must carry the cursor with it, or `F3` silently walks backwards
    /// through results the user has already seen.
    #[test]
    fn chunks_arriving_out_of_order_still_leave_the_matches_in_row_order() {
        let mut finder = Finder::default();
        finder.absorb(vec![hit(500, 0), hit(500, 30), hit(700, 0)]);
        finder.current = Some(0); // the user is standing on row 500's first hit
        finder.absorb(vec![hit(100, 0), hit(200, 0)]);
        finder.absorb(vec![hit(9_000, 0)]);
        finder.absorb(vec![hit(600, 12)]);

        let rows: Vec<u64> = finder.matches.iter().map(|m| m.line).collect();
        assert_eq!(rows, vec![100, 200, 500, 500, 600, 700, 9_000]);
        assert_eq!(
            finder.current,
            Some(2),
            "the cursor must still point at row 500's first hit"
        );
        assert_eq!(finder.matches[finder.current.unwrap()].line, 500);
    }

    /// A fresh search shows the next match **from where the user is**, and stepping wraps.
    #[test]
    fn stepping_starts_from_the_viewport_and_wraps_at_both_ends() {
        let mut finder = Finder::default();
        finder.absorb(vec![hit(10, 0), hit(4_000, 0), hit(4_100, 0)]);

        // Reading around row 4,000 in a big file: the first thing shown is 4,000, not 10.
        finder.from_row = 3_500;
        assert_eq!(finder.step(true), Some(4_000));
        assert_eq!(finder.step(true), Some(4_100));
        assert_eq!(finder.step(true), Some(10), "forwards must wrap to the top");
        assert_eq!(
            finder.step(false),
            Some(4_100),
            "backwards must wrap to the bottom"
        );

        // And with nothing below the viewport, the first step wraps rather than doing nothing.
        let mut late = Finder {
            from_row: 9_000,
            ..Finder::default()
        };
        late.absorb(vec![hit(10, 0), hit(20, 0)]);
        assert_eq!(late.step(true), Some(10));

        assert_eq!(
            Finder::default().step(true),
            None,
            "no matches, no movement"
        );
    }

    /// The current match is a different colour from the rest, which is the whole of stepping being
    /// visible — and only this row's matches are returned.
    #[test]
    fn only_this_rows_matches_are_spans_and_the_current_one_stands_out() {
        let mut finder = Finder::default();
        finder.absorb(vec![hit(7, 4), hit(7, 40), hit(8, 0)]);
        finder.current = Some(1);

        let mut spans = Vec::new();
        finder.spans(7, &mut spans);
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].start, spans[0].end), (4, 9));
        assert_eq!(spans[0].bg, Some(MATCH_BG));
        assert_eq!(spans[1].bg, Some(CURRENT_MATCH_BG));
        assert_eq!(spans[1].fg, Some(CURRENT_MATCH_INK));

        // `out` is reused across rows, so a row with no matches must clear what the last one left.
        finder.spans(9, &mut spans);
        assert!(spans.is_empty(), "a row with no matches left spans behind");
    }

    /// **A non-BMP character typed into the query survives being two `WM_CHAR` messages.**
    ///
    /// Pushed separately they become two replacement characters, and the search then fails to find
    /// the thing that is in the file — as "no matches", which is indistinguishable from the truth.
    #[test]
    fn a_surrogate_pair_becomes_one_character_in_the_query() {
        let mut query = String::new();
        let mut pending = None;
        for unit in "ok 🦅".encode_utf16() {
            assert!(push_typed_unit(&mut query, &mut pending, unit));
        }
        assert_eq!(query, "ok 🦅");

        // Control codes are keys, not text: `Ctrl+F` arrives here as 0x06 and `Enter` as 0x0D.
        assert!(!push_typed_unit(&mut query, &mut pending, 0x06));
        assert!(!push_typed_unit(&mut query, &mut pending, 0x0D));
        assert_eq!(query, "ok 🦅");

        // A lone high surrogate is not silently dropped — a character that vanishes as it is typed
        // looks like a keyboard fault.
        assert!(push_typed_unit(&mut query, &mut pending, 0xD83E));
        assert!(push_typed_unit(&mut query, &mut pending, u16::from(b'x')));
        assert_eq!(query, "ok 🦅\u{FFFD}x");
    }

    /// Everything §7.4 obliges a search to disclose has to be *sayable*, and the title is the only
    /// place there is to say it.
    #[test]
    fn the_title_names_every_way_a_search_can_end() {
        let mut finder = Finder {
            query: "timeout".to_owned(),
            ..Finder::default()
        };
        finder.absorb(vec![hit(4, 0), hit(9, 0)]);
        finder.current = Some(1);
        finder.outcome = Some(Outcome::Capped);
        finder.truncated = 3;

        let text = finder.describe().expect("a query is in play");
        assert!(text.contains("2 of 2"), "{text}");
        assert!(text.contains("capped"), "{text}");
        assert!(text.contains("3 lines too slow"), "{text}");

        // A pattern that would not compile must not read as "searched, found nothing".
        let refused = Finder {
            query: "(".to_owned(),
            error: Some("unclosed group".to_owned()),
            ..Finder::default()
        };
        let text = refused.describe().expect("an error is in play");
        assert!(text.contains("unclosed group"), "{text}");
        assert!(!text.contains("no matches"), "{text}");

        // And with no query at all there is nothing to say.
        assert!(Finder::default().describe().is_none());
    }

    /// A search over a real file, through the document, ending on screen as spans.
    ///
    /// This is the join the unit tests above cannot make: `Document::find` snapshots the set, a
    /// worker searches it, `Finder::poll` collects the results, and `row_spans` — the method the
    /// painter calls — returns them for the right rows.
    #[test]
    fn a_document_search_reaches_the_rows_the_painter_asks_about() {
        let path = scratch_log("tailhawk_find_test.log", 400);
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));

        doc.finder.query = "line 137 ".to_owned();
        doc.find();

        // The worker is a worker: this is the shell's timer tick, without the timer.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.finder.running.is_some() && std::time::Instant::now() < deadline {
            doc.poll_find();
            std::thread::yield_now();
        }
        doc.poll_find();

        assert_eq!(doc.finder.outcome, Some(Outcome::Complete));
        assert_eq!(
            doc.finder.matches.len(),
            1,
            "\"line 137 \" occurs once in the fixture"
        );
        assert_eq!(doc.finder.matches[0].line, 137);
        assert_eq!(
            doc.finder.current,
            Some(0),
            "the match should have been stepped to as it arrived"
        );

        // And the painter's question is answered for that row and no other. A match is the span
        // with a background; the semantic layer beneath it only ever sets ink.
        let mut spans = Vec::new();
        doc.row_spans(137, &mut spans);
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].bg, Some(CURRENT_MATCH_BG));
        assert_eq!(spans.iter().filter(|s| s.bg.is_some()).count(), 1);
        doc.row_spans(136, &mut spans);
        assert!(spans.iter().all(|s| s.bg.is_none()), "row 136 has no match");

        // The view moved to it: row 137 is not in the first screenful of 20 rows.
        assert!(
            doc.view.grid().scroll().row > 100,
            "the view did not follow the match, it is at row {}",
            doc.view.grid().scroll().row
        );

        let _ = std::fs::remove_file(&path);
    }

    /// §7.1's layer order, on a real file: the semantic catalogue colours a row on its own, and a
    /// search's match sits **over** it — the match keeps its background where the two want the
    /// same characters, and the catalogue keeps everything the match does not cover.
    #[test]
    fn the_semantic_layer_colours_rows_and_a_match_sits_over_it() {
        let path = std::env::temp_dir().join("tailhawk_semantic_test.log");
        std::fs::write(
            &path,
            "2026-08-16 09:14:02.117 INFO  Api.Controller returned 412 rows in 88ms\n\
             2026-08-16 09:14:02.118 ERROR Api.Dispatch job 41982 failed\n",
        )
        .expect("write the fixture");
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        doc.highlighter.begin_frame();

        // With no search, the catalogue alone: timestamp, level, number, duration.
        let mut spans = Vec::new();
        doc.row_spans(1, &mut spans);
        let inks: Vec<_> = spans.iter().map(|s| (s.start, s.end, s.fg, s.bg)).collect();
        assert_eq!(inks[0], (0, 23, Some(semantic::TIMESTAMP), None));
        // The row is presented in columns (the fixture detects as timestamped text), so the level
        // sits where the layout put it rather than at its raw offset.
        let e = doc.row_text(1).expect("row").find("ERROR").expect("level");
        assert_eq!(inks[1], (e, e + 5, Some(semantic::ERROR), None));
        assert!(spans.iter().all(|s| s.bg.is_none()));

        doc.finder.query = "ERROR".to_owned();
        doc.find();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.finder.running.is_some() && std::time::Instant::now() < deadline {
            doc.poll_find();
            std::thread::yield_now();
        }
        doc.poll_find();
        assert_eq!(doc.finder.matches.len(), 1);

        // Now the match owns `ERROR` — its background *and* its ink — and the timestamp before it
        // and the number after it are still the catalogue's.
        doc.row_spans(1, &mut spans);
        let inks: Vec<_> = spans.iter().map(|s| (s.start, s.end, s.fg, s.bg)).collect();
        assert_eq!(inks[0], (0, 23, Some(semantic::TIMESTAMP), None));
        assert_eq!(
            inks[1],
            (e, e + 5, Some(CURRENT_MATCH_INK), Some(CURRENT_MATCH_BG))
        );
        assert!(
            inks.iter()
                .any(|&(_, _, fg, _)| fg == Some(semantic::NUMBER)),
            "41982 is still a number: {inks:?}"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// **A rotation that renumbers rows throws the matches away**, because a match is a row number
    /// and a byte range inside it — both of which a truncate invalidates while the count stays put.
    #[test]
    fn matches_do_not_survive_a_source_that_renumbers_its_rows() {
        let path = scratch_log("tailhawk_find_truncate_test.log", 50);
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        doc.finder.absorb(vec![hit(10, 0)]);
        doc.finder.current = Some(0);

        // §5.5's copy-truncate: same name, same handle, different bytes from offset zero.
        std::fs::write(&path, "a totally different first line\n").expect("truncate the fixture");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !doc.finder.matches.is_empty() && std::time::Instant::now() < deadline {
            doc.poll_follow();
            std::thread::yield_now();
        }

        assert!(
            doc.finder.matches.is_empty(),
            "matches addressing the old bytes survived the truncate"
        );
        assert_eq!(doc.finder.current, None);

        let _ = std::fs::remove_file(&path);
    }

    /// Adds a chip the way the keys do and waits for the worker, as the timer would.
    fn filter_for(doc: &mut Document, text: &str, polarity: Polarity) {
        doc.add_chip(text, polarity);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.filtering.running.is_some() && std::time::Instant::now() < deadline {
            doc.poll_filter();
            std::thread::yield_now();
        }
        doc.poll_filter();
    }

    /// §7.3's in-place hide, end to end on a real file: the grid counts survivors, view row *k*
    /// reads as file row `kept[k]`, an exclude composes with the include, and clearing the chips
    /// gives the whole file back.
    #[test]
    fn a_filtered_document_shows_only_the_rows_that_survive() {
        let path = std::env::temp_dir().join("tailhawk_filter_test.log");
        let text: String = (0..300)
            .map(|i| {
                if i % 50 == 0 {
                    format!("ERROR line {i} failed\n")
                } else if i % 50 == 25 {
                    format!("ERROR line {i} retrying\n")
                } else {
                    format!("INFO line {i} fine\n")
                }
            })
            .collect();
        std::fs::write(&path, text).expect("write the fixture");
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));

        filter_for(&mut doc, "error", Polarity::Include);
        assert_eq!(doc.filtering.outcome, Some(Outcome::Complete));
        assert_eq!(doc.filtering.kept.len(), 12, "six failed, six retrying");
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.view.grid().total_rows(), 12);
        assert_eq!(doc.row_text(0), Some("ERROR line 0 failed"));
        assert_eq!(doc.row_text(1), Some("ERROR line 25 retrying"));
        assert_eq!(doc.row_text(11), Some("ERROR line 275 retrying"));
        assert_eq!(doc.row_text(12), None, "past the survivors");
        assert!(
            doc.describe().contains("▼ +error · 12 of 300"),
            "{}",
            doc.describe()
        );

        filter_for(&mut doc, "retrying", Polarity::Exclude);
        assert_eq!(doc.filtering.kept.len(), 6);
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.row_text(1), Some("ERROR line 50 failed"));
        assert!(
            doc.describe().contains("▼ +error −retrying · 6 of 300"),
            "{}",
            doc.describe()
        );

        // A search's matches are file rows; stepping to one lands on its view row.
        doc.finder.query = "line 100".to_owned();
        doc.find();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.finder.running.is_some() && std::time::Instant::now() < deadline {
            doc.poll_find();
            std::thread::yield_now();
        }
        doc.poll_find();
        assert_eq!(doc.finder.matches.len(), 1);
        assert_eq!(doc.finder.matches[0].line, 100, "a file row");
        let mut spans = Vec::new();
        doc.row_spans(2, &mut spans);
        assert!(
            spans.iter().any(|s| s.bg == Some(CURRENT_MATCH_BG)),
            "view row 2 is file row 100 and wears the match"
        );

        doc.clear_filter();
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.view.grid().total_rows(), 300);
        // A view at the bottom of six rows is following, and stays following into three hundred.
        assert!(doc.view.grid().is_following());
        assert_eq!(doc.row_text(299), Some("INFO line 299 fine"));
        assert_eq!(doc.row_text(298), Some("INFO line 298 fine"));
        assert!(doc.describe().contains("▸ "), "{}", doc.describe());

        let _ = std::fs::remove_file(&path);
    }

    /// A chip that does not parse is held in the title and changes nothing else.
    #[test]
    fn a_chip_that_does_not_parse_is_reported_and_the_view_stands() {
        let path = scratch_log("tailhawk_filter_bad_chip.log", 20);
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        filter_for(&mut doc, "/[unclosed/", Polarity::Include);
        assert!(!doc.filtering.active());
        assert!(doc.filtering.error.is_some());
        assert!(
            doc.describe().contains("▼ /[unclosed/"),
            "{}",
            doc.describe()
        );
        assert_eq!(doc.view.grid().total_rows(), 20);
        let _ = std::fs::remove_file(&path);
    }

    /// Lines appended after the pass are sieved too, and a truncate throws the survivors away and
    /// starts over against what the file now is.
    #[test]
    fn a_filter_follows_growth_and_survives_a_truncate_by_starting_over() {
        let path = std::env::temp_dir().join("tailhawk_filter_growth.log");
        std::fs::write(&path, "INFO a\nERROR b\nINFO c\n").expect("write");
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        filter_for(&mut doc, "error", Polarity::Include);
        assert_eq!(doc.filtering.kept, [1]);

        {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("append");
            writeln!(f, "ERROR d\nINFO e\nERROR f").expect("write");
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.filtering.kept.len() < 3 && std::time::Instant::now() < deadline {
            doc.poll_follow();
            doc.poll_filter();
            std::thread::yield_now();
        }
        assert_eq!(doc.filtering.kept, [1, 3, 5], "growth was sieved");
        assert_eq!(doc.filtering.covered, 6);

        // §5.5's copy-truncate: the rows are different bytes at the same numbers.
        std::fs::write(&path, "ERROR only\n").expect("truncate");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.filtering.kept != [0] && std::time::Instant::now() < deadline {
            doc.poll_follow();
            doc.poll_filter();
            std::thread::yield_now();
        }
        assert_eq!(doc.filtering.kept, [0], "started over on the new bytes");
        let _ = std::fs::remove_file(&path);
    }
    /// §6.3 end to end: a Serilog file is detected on open, the title says so, and a field chip
    /// — `level >= Warning` — filters through the parsed record. Before M6 that chip evaluated
    /// to unknown on every row.
    #[test]
    fn a_detected_format_names_itself_and_makes_field_chips_work() {
        let path = std::env::temp_dir().join("tailhawk_detect_test.log");
        let text: String = (0..120)
            .map(|i| {
                let level = match i % 4 {
                    0 => "ERR",
                    1 => "WRN",
                    _ => "INF",
                };
                format!(
                    "2026-08-16 09:14:{:02}.117 +02:00 [{level}] line {i}\n",
                    i % 60
                )
            })
            .collect();
        std::fs::write(&path, text).expect("write the fixture");
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.detection.accepted.map(|f| f.id), Some("serilog-file"));
        assert!(
            doc.describe().contains("· Serilog (file) "),
            "{}",
            doc.describe()
        );

        filter_for(&mut doc, "level >= Warning", Polarity::Include);
        assert_eq!(doc.filtering.outcome, Some(Outcome::Complete));
        assert_eq!(doc.filtering.kept.len(), 60, "30 ERR + 30 WRN of 120");
        assert_eq!(doc.filtering.kept[0], 0);
        assert_eq!(doc.filtering.kept[1], 1);
        assert_eq!(doc.filtering.kept[2], 4);
        let _ = std::fs::remove_file(&path);
    }
    /// §6.4's collapse: with `records_only`, continuations leave the row space and the record's
    /// first lines remain — composed with a chip when there is one.
    #[test]
    fn collapsing_hides_continuations_and_composes_with_a_chip() {
        let path = std::env::temp_dir().join("tailhawk_collapse_test.log");
        std::fs::write(
            &path,
            "2026-08-16 09:14:02.117 +02:00 [INF] Started\n\
             2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982\n\
             System.InvalidOperationException: boom\n\
                at Api.Dispatch.Run() in Dispatch.cs:line 42\n\
             2026-08-16 09:14:04.002 +02:00 [WRN] Retry 1/3 for job 41982\n",
        )
        .expect("write");
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.detection.accepted.map(|f| f.id), Some("serilog-file"));

        doc.filtering.records_only = true;
        doc.filtering.clear_results();
        doc.refilter();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.filtering.running.is_some() && std::time::Instant::now() < deadline {
            doc.poll_filter();
            std::thread::yield_now();
        }
        doc.poll_filter();
        assert_eq!(
            doc.filtering.kept,
            [0, 1, 4],
            "three records, two frames hidden"
        );
        assert!(doc.describe().contains("▤ records"), "{}", doc.describe());

        filter_for(&mut doc, "job", Polarity::Include);
        assert_eq!(
            doc.filtering.kept,
            [1, 4],
            "the chip composes with the collapse"
        );
        let _ = std::fs::remove_file(&path);
    }
    /// M6's done-criterion: "MEL Simple two-line records assemble correctly". Collapsed, each record
    /// is one row with the next line's text in its message column; uncollapsed, the message is the
    /// row below, indented under the message column.
    #[test]
    fn mel_simple_two_line_records_assemble_when_collapsed() {
        let path = std::env::temp_dir().join("tailhawk_mel_test.log");
        std::fs::write(
            &path,
            "info: Microsoft.Hosting.Lifetime[14]\n      Now listening on: http://localhost:5000\n\
             fail: Api.Dispatch[0]\n      Failed to dispatch job 41982\n\
             info: Microsoft.Hosting.Lifetime[0]\n      Application started.\n",
        )
        .expect("write");
        let mut doc = Document::open(&path).expect("open the fixture");
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.detection.accepted.map(|f| f.id), Some("mel-simple"));
        assert!(doc.header_text().is_some());
        let row1 = doc.row_text(1).expect("row 1").to_owned();
        assert!(row1.trim_start().starts_with("Now listening"), "{row1:?}");
        assert!(
            row1.starts_with("   "),
            "indented under the message column: {row1:?}"
        );

        doc.filtering.records_only = true;
        doc.filtering.clear_results();
        doc.refilter();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while doc.filtering.running.is_some() && std::time::Instant::now() < deadline {
            doc.poll_filter();
            std::thread::yield_now();
        }
        doc.poll_filter();
        doc.lay_out((8.0, 10.0), (800, 200));
        assert_eq!(doc.filtering.kept, [0, 2, 4]);
        let row = doc
            .row_text(1)
            .expect("view row 1 is file row 2")
            .to_owned();
        assert!(
            row.starts_with("fail"),
            "no timestamp column when no line has one: {row:?}"
        );
        assert!(
            row.ends_with("Failed to dispatch job 41982"),
            "the next line's text is the message: {row:?}"
        );
        let _ = std::fs::remove_file(&path);
    }
    /// E11 end to end: an app's own `outputTemplate` beside its log is compiled and wins detection
    /// over the catalogue — a shape no built-in knows becomes columns anyway.
    #[test]
    fn a_template_beside_the_log_is_compiled_and_detected() {
        let dir = std::env::temp_dir().join("tailhawk-template-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("appsettings.json"),
            r#"{"Serilog":{"WriteTo":[{"Name":"File","Args":{"path":"app.log",
               "outputTemplate":"{Timestamp:HH:mm:ss.fff}|{Level:u}|{SourceContext}|{Message:lj}{NewLine}{Exception}"}}]}}"#,
        )
        .expect("config");
        let log = dir.join("app.log");
        std::fs::write(
            &log,
            "09:14:02.117|INFORMATION|Api.Controller|Started\n\
             09:14:03.884|ERROR|Api.Dispatch|Failed to dispatch job 41982\n\
             09:14:04.002|WARNING|Api.Sql|Retry 1/3\n",
        )
        .expect("log");
        let mut doc = Document::open(&log).expect("open");
        doc.lay_out((8.0, 10.0), (800, 200));
        let accepted = doc.detection.accepted.expect("a format");
        assert_eq!(
            accepted.id, "template:appsettings.json",
            "{:?}",
            doc.detection.candidates
        );
        assert!(
            doc.describe().contains("Serilog (appsettings.json)"),
            "{}",
            doc.describe()
        );
        assert_eq!(
            doc.header_text()
                .map(|h| h.trim_start().starts_with("timestamp")),
            Some(true)
        );
        let row = doc.row_text(1).expect("row 1");
        assert!(
            row.contains("ERROR") && row.ends_with("Failed to dispatch job 41982"),
            "{row:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// A headless screenshot: opens `TAILHAWK_SHOT_FILE`, applies `TAILHAWK_SHOT_KEYS` (a `;`-separated
    /// script of `chip:<text>`, `xchip:<text>`, `find:<text>`, `focus:find|chip|grid`, `type:<text>`,
    /// `collapse`) and writes the frame the shell would draw to `TAILHAWK_SHOT_OUT` as a BMP. For a
    /// harness that has no desktop to capture — the offscreen target is the whole point.
    ///
    /// ```text
    /// TAILHAWK_SHOT_FILE=x.log TAILHAWK_SHOT_KEYS="chip:job;focus:find;type:dispatch" TAILHAWK_SHOT_OUT=out.bmp \
    ///   cargo test --release -p tailhawk headless_screenshot -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "needs TAILHAWK_SHOT_FILE and TAILHAWK_SHOT_OUT"]
    fn headless_screenshot() {
        let (Some(file), Some(out)) = (
            std::env::var_os("TAILHAWK_SHOT_FILE"),
            std::env::var_os("TAILHAWK_SHOT_OUT"),
        ) else {
            eprintln!("skipped: set TAILHAWK_SHOT_FILE and TAILHAWK_SHOT_OUT");
            return;
        };
        let mut renderer = match Renderer::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("skipped: no device ({e})");
                return;
            }
        };
        let (w, h) = (1200u32, 500u32);
        let cell = renderer.cell().expect("cell metrics");
        let mut doc = Document::open(std::path::Path::new(&file)).expect("open");
        doc.lay_out(cell, (w, h));
        for step in std::env::var("TAILHAWK_SHOT_KEYS")
            .unwrap_or_default()
            .split(';')
            .filter(|s| !s.is_empty())
        {
            let (verb, arg) = step.split_once(':').unwrap_or((step, ""));
            match verb {
                "chip" => filter_for(&mut doc, arg, Polarity::Include),
                "xchip" => filter_for(&mut doc, arg, Polarity::Exclude),
                "find" => {
                    doc.chrome.find.set_text(arg);
                    doc.finder.query = arg.to_owned();
                    doc.find();
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    while doc.finder.running.is_some() && std::time::Instant::now() < deadline {
                        doc.poll_find();
                        std::thread::yield_now();
                    }
                    doc.poll_find();
                }
                "focus" => {
                    doc.chrome.focus = match arg {
                        "find" => Focus::Find,
                        "chip" => Focus::NewChip,
                        _ => Focus::Grid,
                    }
                }
                "type" => {
                    if let Some(f) = doc.chrome.focused() {
                        f.insert(arg);
                    }
                }
                "tabs" => {
                    let names: Vec<String> = arg.split(',').map(str::to_owned).collect();
                    let active = names.len().saturating_sub(1);
                    doc.tab_strip = (names, active);
                }
                "collapse" => {
                    doc.filtering.records_only = true;
                    doc.filtering.clear_results();
                    doc.refilter();
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                    while doc.filtering.running.is_some() && std::time::Instant::now() < deadline {
                        doc.poll_filter();
                        std::thread::yield_now();
                    }
                    doc.poll_filter();
                }
                other => panic!("unknown step {other:?}"),
            }
        }
        doc.status = format!("hardware — {}", doc.describe());
        doc.lay_out(cell, (w, h));
        doc.highlighter.begin_frame();
        let pixels = renderer.snapshot(w, h, &doc.view, &doc).expect("snapshot");
        write_bmp(std::path::Path::new(&out), &pixels);
        eprintln!("wrote {}", out.to_string_lossy());
    }

    /// A 32-bit BGRA BMP, top-down — no encoder needed, and every viewer opens it.
    fn write_bmp(path: &std::path::Path, pixels: &tailhawk_core::Pixels) {
        let (w, h) = (pixels.width(), pixels.height());
        let mut out = Vec::with_capacity(54 + (w * h * 4) as usize);
        let size = 54 + w * h * 4;
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&(w as i32).to_le_bytes());
        out.extend_from_slice(&(-(h as i32)).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(w * h * 4).to_le_bytes());
        for _ in 0..4 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        for y in 0..h {
            for x in 0..w {
                let [b, g, r, _] = pixels.at(x, y);
                out.extend_from_slice(&[b, g, r, 255]);
            }
        }
        std::fs::write(path, out).expect("write the bmp");
    }
    /// V7's model: the shown tab is what every handler means by the document; cycling wraps; closing
    /// the last leaves an empty shell rather than a dangling index.
    #[test]
    fn tabs_show_one_document_cycle_and_close_safely() {
        let a = scratch_log("tailhawk_tabs_a.log", 10);
        let b = scratch_log("tailhawk_tabs_b.log", 20);
        let mut tabs = Tabs::default();
        assert!(tabs.as_ref().is_none());
        tabs.push(Document::open(&a).expect("a"));
        tabs.push(Document::open(&b).expect("b"));
        assert_eq!(tabs.active, 1, "a new tab is shown");
        assert_eq!(tabs.as_ref().map(|d| d.set.total_rows()), Some(20));
        tabs.cycle(true);
        assert_eq!(tabs.active, 0);
        tabs.cycle(false);
        assert_eq!(tabs.active, 1);
        assert_eq!(tabs.labels().len(), 2);
        assert!(tabs.close_active());
        assert_eq!(tabs.active, 0);
        assert_eq!(tabs.as_ref().map(|d| d.set.total_rows()), Some(10));
        assert!(!tabs.close_active());
        assert!(tabs.as_ref().is_none());
        assert!(!tabs.close_active(), "closing nothing is not an error");
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }
    /// §8.1's watched folder: a directory plus a glob; what matches now is opened, what appears
    /// later is adopted, and nothing is adopted twice.
    #[test]
    fn a_watched_folder_adopts_new_matching_files_once() {
        assert!(Watch::matches("*.log", "app.log"));
        assert!(
            Watch::matches("*.LOG", "app.log"),
            "case-insensitive, as Windows names are"
        );
        assert!(Watch::matches("app-??.log", "app-01.log"));
        assert!(!Watch::matches("*.log", "app.txt"));
        assert!(!Watch::matches("app-??.log", "app-1.log"));

        let dir = std::env::temp_dir().join("tailhawk-watch-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(dir.join("a.log"), "one\n").expect("a");
        std::fs::write(dir.join("notes.txt"), "no\n").expect("txt");
        let mut watch = Watch::from_arg(&dir).expect("a directory is a watch");
        assert_eq!(watch.pattern, "*.log");
        let first = watch.new_files();
        assert_eq!(first.len(), 1);
        assert!(first[0].ends_with("a.log"));
        assert!(watch.new_files().is_empty(), "nothing new");
        std::fs::write(dir.join("b.log"), "two\n").expect("b");
        let next = watch.new_files();
        assert_eq!(next.len(), 1);
        assert!(next[0].ends_with("b.log"));

        let glob = Watch::from_arg(&dir.join("*.txt")).expect("a glob is a watch");
        assert_eq!(glob.pattern, "*.txt");
        assert!(
            Watch::from_arg(&dir.join("a.log")).is_none(),
            "a plain file is not"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    /// §12.4's per-file state: what a document is looked at through survives a round trip through
    /// the settings and comes back on the next open.
    #[test]
    fn a_files_chips_and_collapse_are_remembered_and_restored() {
        let path = scratch_log("tailhawk_settings_test.log", 40);
        let mut doc = Document::open(&path).expect("open");
        doc.lay_out((8.0, 10.0), (800, 200));
        assert!(
            doc.file_state()
                .is_some_and(|s| s.chips.is_empty() && !s.collapse),
            "nothing to remember yet"
        );
        filter_for(&mut doc, "line 1", Polarity::Include);
        filter_for(&mut doc, "line 12", Polarity::Exclude);
        let state = doc.file_state().expect("chips to remember");
        assert_eq!(state.chips, ["+line 1", "-line 12"]);
        assert!(!state.collapse);

        let mut settings = settings::Settings::default();
        settings.set_file(state);
        let text = settings.to_toml();
        let back = settings::Settings::from_toml(&text);
        let remembered = back
            .file(&path.to_string_lossy())
            .expect("the file is in the settings")
            .clone();

        let mut again = Document::open(&path).expect("open again");
        again.lay_out((8.0, 10.0), (800, 200));
        again.apply_state(&remembered);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while again.filtering.running.is_some() && std::time::Instant::now() < deadline {
            again.poll_filter();
            std::thread::yield_now();
        }
        again.poll_filter();
        assert_eq!(again.filtering.chips.chips.len(), 2);
        assert_eq!(again.filtering.chips.chips[1].polarity, Polarity::Exclude);
        assert_eq!(doc.filtering.kept, again.filtering.kept, "the same view");
        let _ = std::fs::remove_file(&path);
    }
}
