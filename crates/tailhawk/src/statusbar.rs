//! The status bar, as the real Windows control — `UI-DESIGN.md` §1.1.
//!
//! **This replaces a drawn band, and the reason is not only that it looked wrong.** The old one
//! rendered its text through the painter's chrome atlas and was *losing glyphs*: `hardware` came
//! out `har ware`, `following` as `fo owing`, `tailhawk-spill` as `tai hawk-spi`, `probably` as
//! `probab y` — every missing character an `l` or a `d` — while the same letters drew correctly in
//! the grid a few pixels above. A surface with its own text stack is a surface with its own bugs,
//! and this one sat on screen unnoticed until the owner reported it. There is nothing here that
//! draws, so there is nothing here that can lose a letter.
//!
//! **§1.1's table is amended alongside this.** It rejected a "fixed-function status bar" in favour
//! of a row of live, clickable chips — which is the decision that produced the drawn band. A real
//! status bar takes **parts** and hit-tests them, so the chips remain available as parts of a
//! control rather than as drawing.
//!
//! **The parts arrived on 2026-09-21, and the composed sentence went with them.** The owner, of
//! the one long line the bar used to carry: it "is meaningless to me. It should be more like a
//! conventional status bar, not just a debugging bar." [`StatusPanes`] is what it says now and
//! [`status_panes_of`] is the pure mapping that decides it — message, position, find, filter,
//! view, format, encoding and tail, each silent when it has nothing to report. The GPU driver
//! name is gone; the frame instrument keeps its way back through `TAILHAWK_FRAME_STATS`, on the
//! message pane.
//!
//! **Every fact the sentence carried still reaches the screen, and that took a second pass.** The
//! first cut of the panes had nowhere for the Loki lag, the export progress, the sort, the
//! revealed invisibles or the High Contrast warning, and rendered a paused tail as an empty pane
//! — so wiring it up would have deleted all of them from the UI while `Document::describe` went
//! on computing them into a field nothing read. Panes are a decision about what is worth showing;
//! a view-model with no field for a fact has made that decision by accident.
//!
//! **The band height is the system's**, read back from the control. It follows the shell font and
//! the DPI, and the layout that has to make room for it is not the place to decide how tall
//! Windows draws its own bar.

use crate::tabstrip::shell_font;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetTextExtentPoint32W, ReleaseDC, SelectObject, HFONT, HGDIOBJ,
};
use windows::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_BAR_CLASSES, INITCOMMONCONTROLSEX, SBARS_SIZEGRIP, SB_SETPARTS,
    SB_SETTEXTW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetWindowRect, SendMessageW, HMENU,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT, WM_SIZE, WS_CHILD, WS_CLIPSIBLINGS, WS_VISIBLE,
};

/// The one part spans the whole bar. `-1` is the documented "to the right edge" width.
const TO_THE_EDGE: i32 = -1;

/// The window's status bar.
pub struct StatusBar {
    hwnd: HWND,
    font: HFONT,
    /// What the bar was last told to say. **Kept so it is not told again**: the shell composes its
    /// status every frame, and `SB_SETTEXTW` with unchanged text still invalidates and repaints
    /// the bar, which is a repaint per frame for a string that changes a few times a second.
    shown: StatusPanes,
    /// The client width the parts were last laid out for, so a frame that resized nothing does no
    /// GDI work. See [`StatusBar::resize`].
    laid_out_for: Option<i32>,
}

/// The air around a pane's text, in pixels, so the words do not touch the dividers.
const PANE_PADDING: i32 = 14;

/// Where each part's right edge goes, given the bar's width and what each pane's text measures.
///
/// **Pure, and extracted for the reason this project extracts everything.** This is arithmetic
/// wearing Win32 clothing: it needs no window, and while it sat inside the `SendMessage` that
/// consumes it there was no way to test the case that breaks it. `SB_SETPARTS` requires
/// **non-decreasing** right edges, and a naive running sum violates that the moment the trailing
/// panes are collectively wider than the bar — a narrow window with a long format name and a long
/// filter — leaving the rightmost panes with edges past the client rectangle and comctl32 asked
/// for a part that ends before the one before it.
///
/// So the trailing panes are given what there is: each takes its measured width or the room that
/// remains, whichever is less, and the message pane keeps whatever is left over — which is
/// nothing, in the case that used to produce a malformed array. The last edge is always
/// [`TO_THE_EDGE`], so no rounding difference can leave a sliver of the bar undrawn.
fn edges_of(total: i32, widths: [i32; StatusPanes::COUNT]) -> [i32; StatusPanes::COUNT] {
    let total = total.max(0);
    let trailing: i32 = widths[1..].iter().sum();
    let mut edges = [0i32; StatusPanes::COUNT];
    let mut x = (total - trailing).max(0);
    edges[0] = x;
    for i in 1..StatusPanes::COUNT {
        x = (x + widths[i]).min(total);
        edges[i] = x;
    }
    edges[StatusPanes::COUNT - 1] = TO_THE_EDGE;
    edges
}

impl StatusBar {
    /// Creates the control as a child of `parent`. `None` when it could not be created, in which
    /// case the window simply has no status bar — the same way the toolbar and the tab strip fail.
    pub fn new(parent: HWND, instance: HINSTANCE) -> Option<StatusBar> {
        let init = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_BAR_CLASSES,
        };
        unsafe {
            let _ = InitCommonControlsEx(&init);
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("msctls_statusbar32"),
                PCWSTR::null(),
                WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0 | SBARS_SIZEGRIP),
                0,
                0,
                0,
                0,
                parent,
                HMENU::default(),
                instance,
                None,
            )
        }
        .ok()?;
        let font = shell_font();
        if !font.is_invalid() {
            unsafe {
                SendMessageW(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
        }
        // The same theme decision the menus take — `controls::apply_theme`, one place for all.
        crate::header::trace(&format!("child: statusbar hwnd={:?}", hwnd.0));
        crate::controls::apply_theme(hwnd, tailhawk_core::theme::theme().dark);
        let bar = StatusBar {
            hwnd,
            font,
            shown: StatusPanes::default(),
            laid_out_for: None,
        };
        bar.apply_parts();
        Some(bar)
    }

    /// Lays the parts out from what the bar is currently saying, and fills them.
    ///
    /// **The trailing panes are measured, and the first takes what is left.** A status bar's
    /// right-hand panes are facts of fixed shape — an encoding, a format name, a line number — and
    /// sizing them to their own text is what stops `Line 1,192 of 419,501` being clipped on one
    /// document and swimming in space on another. The message pane is first and absorbs the
    /// remainder, which is where a long notice has room to be read.
    ///
    /// A pane with nothing to say measures zero and collapses, so the dividers a reader sees are
    /// only the ones between panes that are saying something.
    fn apply_parts(&self) {
        let texts = self.shown.parts();
        let widths = self.measure(&texts);
        let mut rc = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rc) }.is_err() {
            return;
        }
        let edges = edges_of((rc.right - rc.left).max(0), widths);
        unsafe {
            SendMessageW(
                self.hwnd,
                SB_SETPARTS,
                WPARAM(StatusPanes::COUNT),
                LPARAM(edges.as_ptr() as isize),
            );
        }
        for (i, text) in texts.iter().enumerate() {
            let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                SendMessageW(
                    self.hwnd,
                    SB_SETTEXTW,
                    WPARAM(i),
                    LPARAM(wide.as_ptr() as isize),
                );
            }
        }
    }

    /// How wide each pane's text is, in pixels, with its padding — zero for an empty one.
    fn measure(&self, texts: &[&str; StatusPanes::COUNT]) -> [i32; StatusPanes::COUNT] {
        let mut out = [0i32; StatusPanes::COUNT];
        let dc = unsafe { GetDC(self.hwnd) };
        if dc.is_invalid() {
            return out;
        }
        let old = if self.font.is_invalid() {
            HGDIOBJ::default()
        } else {
            unsafe { SelectObject(dc, HGDIOBJ(self.font.0)) }
        };
        for (i, text) in texts.iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            let wide: Vec<u16> = text.encode_utf16().collect();
            let mut size = SIZE::default();
            if unsafe { GetTextExtentPoint32W(dc, &wide, &mut size) }.as_bool() {
                out[i] = size.cx + PANE_PADDING;
            }
        }
        unsafe {
            if !old.is_invalid() {
                SelectObject(dc, old);
            }
            ReleaseDC(self.hwnd, dc);
        }
        out
    }

    /// The control's window, for the shell to re-theme when the theme changes.
    /// Re-measures the shell font for `dpi` and gives it to the control.
    ///
    /// **A control keeps the font it was created with**, and this window is created on one monitor
    /// and dragged to another: without this, moving to a 150 % display left every native child
    /// drawing at 100 % beside a grid that had rescaled. `WM_DPICHANGED` is the moment to ask
    /// again, and `SystemParametersInfoForDpi` is what makes the answer per-monitor.
    pub fn set_font(&mut self, dpi: u32) {
        let font = crate::tabstrip::shell_font_for(dpi);
        if font.is_invalid() {
            return;
        }
        unsafe {
            SendMessageW(self.hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
        }
        // The old one goes only after the control has been told about the new one: a GDI object
        // still selected into a live device context is undefined rather than merely untidy.
        if !self.font.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
        self.font = font;
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// How tall the bar is, in client pixels — the system's answer, not ours. The caller subtracts
    /// this from the client area so the grid and its scroll bar end above the bar instead of
    /// beside it.
    pub fn band_height(&self) -> i32 {
        let mut rc = RECT::default();
        if unsafe { GetWindowRect(self.hwnd, &mut rc) }.is_err() {
            return 0;
        }
        (rc.bottom - rc.top).max(0)
    }

    /// Docks the bar along the bottom of its parent's client area.
    ///
    /// **A status bar positions itself**, which is why this forwards `WM_SIZE` rather than calling
    /// `SetWindowPos`: the control reads the parent's client rectangle and takes the bottom strip,
    /// including the corner the size grip needs. Placing it by hand is how a status bar ends up a
    /// pixel out from the window it belongs to.
    /// **The parts are re-laid only when the bar's width actually changed.**
    ///
    /// `WM_SIZE` is cheap and the control ignores it when nothing moved, but `apply_parts` is a
    /// GDI measuring pass and nine messages, and `SB_SETTEXTW` invalidates even when the text is
    /// identical — which is what [`StatusBar::shown`] exists to prevent. This runs every frame,
    /// so doing that work unconditionally would have put a full repaint of the bar into every
    /// paint of a tailing window and made the guard beside it meaningless.
    pub fn resize(&mut self) {
        unsafe {
            SendMessageW(self.hwnd, WM_SIZE, WPARAM(0), LPARAM(0));
        }
        let mut rc = RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rc) }.is_err() {
            return;
        }
        let width = (rc.right - rc.left).max(0);
        if self.laid_out_for == Some(width) {
            return;
        }
        self.laid_out_for = Some(width);
        self.apply_parts();
    }

    /// Sets what the bar says, if it is not already saying it.
    pub fn set_panes(&mut self, panes: &StatusPanes) {
        if self.shown == *panes {
            return;
        }
        self.shown = panes.clone();
        self.apply_parts();
    }
}

impl Drop for StatusBar {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            if !self.font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
        }
    }
}

/// A number a person can read at a glance: `1,204,915`.
///
/// **The status bar is where this lives now, and the detail window's title borrows it.** Every
/// count a reader is meant to take in at a glance is on the bar — the row they are on, the rows a
/// filter kept, the columns the layout is showing — and the record title is the one other place
/// that wants the same treatment. One copy, on the surface that uses it most.
pub(crate) fn with_separators(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (at, c) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What a find is doing, as the bar reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FindFacts {
    /// Which match the caret is on, counting from one; `None` before the first is stepped to.
    pub current: Option<usize>,
    pub total: usize,
    pub running: bool,
}

/// What the tail is doing — the state the last pane reports.
///
/// **A tail tool's most-wanted behaviour, and the reason this is an enum.** `UI-DESIGN.md` §12
/// calls resuming a paused follow "the single most-wanted behaviour in every tail tool", which
/// means *paused* has to be as visible as *following* rather than being its absence. A `bool`
/// renders "not following" as an empty pane, and an empty pane is not an affordance.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tail<'a> {
    /// A sort holds the view still, so it is neither following nor paused — E22's rule.
    #[default]
    Held,
    /// At the tail. `lag` is `tail::lag_text` for a remote source — §4's "the lag is stated",
    /// without which a source that has gone quiet and one we cannot keep up with look identical.
    Following { lag: Option<&'a str> },
    /// Scrolled back. The way forward is named, because §12 says it must be.
    Paused,
    /// A pipe that reached its end is not paused, it is done.
    Complete,
}

/// What one frame knows, as the status bar needs it.
///
/// **A struct of facts rather than a list of arguments**, because the bar has eight panes and a
/// call with eight positional `Option`s is a call whose arguments get transposed silently. Every
/// field is what it is called; nothing here is pre-formatted.
///
/// **Every fact the composed sentence carried has a home here, and that is not decoration.** The
/// first cut of this struct had no `lag`, no `tee`, no `sorted` and a `bool` where [`Tail`] is —
/// so wiring it up silently took the Loki lag, the export progress, the sort and the paused
/// affordance off the screen altogether, while `Document::describe` went on computing them for a
/// field nobody read. A review caught it. A pane can decide a fact is not worth showing; a facts
/// struct with nowhere to put it has made that decision by accident.
#[derive(Clone, Copy, Debug, Default)]
pub struct StatusFacts<'a> {
    /// The most recent thing that happened — an error, a format saved, a source opening.
    pub notice: Option<&'a str>,
    /// An export or a live tee, with its count — E21's "the user asked for a file and this is
    /// where they see it filling".
    pub tee: Option<&'a str>,
    /// The highlight rules that would not compile, already worded.
    pub rules: Option<&'a str>,
    /// §11.2: under High Contrast the user's highlight rules are off, and the bar says so.
    pub contrast: bool,
    /// Whether a document is open at all. With none, the bar says so and every other pane is bare.
    pub open: bool,
    /// The physical row the caret is on, counting from one, and the rows in the set.
    pub caret_row: Option<u64>,
    pub total_rows: u64,
    pub find: Option<FindFacts>,
    /// Rows a filter kept, of the rows there are. **Set only when a filter is in play** — a sort
    /// re-orders the same rows and keeps all of them, and reporting `Filtered 0 of N` for one is
    /// a lie about the document.
    pub filter: Option<(u64, u64)>,
    /// Columns shown, of the columns the format has.
    pub columns: Option<(usize, usize)>,
    /// The sort in force, already worded by `Filtering::describe_sort`.
    pub sorted: Option<&'a str>,
    /// §6.4's revealed invisibles, which change what every row looks like.
    pub invisibles: bool,
    pub format: Option<&'a str>,
    pub encoding: Option<&'a str>,
    pub tail: Tail<'a>,
    /// `LOKI.md` §6: the source answer was cut, so every count over it is a floor, not a total,
    /// and the advisory stands for as long as the document does rather than scrolling away.
    pub cut: bool,
}

/// One frame's status bar, as the eight parts it draws.
///
/// **Panes, not a sentence.** The bar carried one composed line — `● following · current to 30s
/// ago — agent.log: UTF-8 — 1 file — agent.log · timestamped text 100%, 1192 lines, 419501 bytes`
/// — which is an instrument readout: everything the program knew, joined with dashes, in an order
/// driven by how the string was built rather than by what a reader looks for. The owner's report
/// of 2026-09-21 was that it "is meaningless to me. It should be more like a conventional status
/// bar, not just a debugging bar."
///
/// A pane that has nothing to say is **empty**, not absent: the parts are fixed so the eye learns
/// where to look, and a find that is not running simply leaves its pane blank rather than shifting
/// every pane to its right.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusPanes {
    /// What is happening: the latest notice, an export filling, and the warnings that stand.
    pub message: String,
    pub position: String,
    pub find: String,
    pub filter: String,
    /// How the rows are being *shown* — columns hidden, a sort, revealed invisibles. Three facts
    /// of one kind, which is why they share a pane rather than each taking one and leaving it
    /// blank almost always.
    pub view: String,
    pub format: String,
    pub encoding: String,
    /// Following, paused, or done — and for a remote source, how far behind.
    pub tail: String,
}

impl StatusPanes {
    /// How many panes the bar has. Named so the arithmetic that lays them out cannot drift from
    /// the struct that holds them.
    pub const COUNT: usize = 8;

    /// The panes in the order they are drawn, left to right.
    pub fn parts(&self) -> [&str; Self::COUNT] {
        [
            &self.message,
            &self.position,
            &self.find,
            &self.filter,
            &self.view,
            &self.format,
            &self.encoding,
            &self.tail,
        ]
    }
}

/// The model → view-model mapping for the status bar: facts in, eight panes out.
pub fn status_panes_of(facts: StatusFacts<'_>) -> StatusPanes {
    // **What just happened, then what is happening, then what is wrong — and all of them, not
    // whichever was checked first.** The composed sentence gave the notice and the rules warning
    // separate slots and showed both; a first cut of this took `notice.or(rules)`, which made a
    // transient notice hide a standing warning for as long as it showed.
    let mut said: Vec<&str> = Vec::new();
    said.extend(facts.notice);
    said.extend(facts.tee);
    if facts.cut {
        said.push("⚠ answers cut at the limit — narrow the time range or the selector");
    }
    said.extend(facts.rules);
    if facts.contrast {
        said.push("⚑ High Contrast — highlight rules off");
    }
    let message = if said.is_empty() {
        "Ready".to_owned()
    } else {
        said.join(" · ")
    };
    if !facts.open {
        return StatusPanes {
            message,
            ..StatusPanes::default()
        };
    }

    let floor = |n: u64| {
        if facts.cut {
            format!("at least {}", with_separators(n))
        } else {
            with_separators(n)
        }
    };

    let position = match facts.caret_row {
        Some(row) => format!(
            "Line {} of {}",
            with_separators(row),
            floor(facts.total_rows)
        ),
        None => String::new(),
    };

    let find = match facts.find {
        None => String::new(),
        Some(f) if f.total == 0 && f.running => "Searching…".to_owned(),
        Some(f) if f.total == 0 => "No matches".to_owned(),
        Some(f) if f.running => format!(
            "{} {} so far",
            floor(f.total as u64),
            if f.total == 1 { "match" } else { "matches" }
        ),
        Some(f) => match f.current {
            Some(current) => format!("Match {current} of {}", floor(f.total as u64)),
            None => format!(
                "{} {}",
                floor(f.total as u64),
                if f.total == 1 { "match" } else { "matches" }
            ),
        },
    };

    let filter = match facts.filter {
        Some((kept, total)) => format!("Filtered {} of {}", with_separators(kept), floor(total)),
        None => String::new(),
    };

    // A pane that reports "8 of 8" is a pane reporting nothing, and the owner's complaint about
    // this bar was that it said everything it knew rather than what was worth knowing.
    let mut view: Vec<String> = Vec::new();
    if let Some((shown, total)) = facts.columns {
        if shown < total {
            view.push(format!("{shown} of {total} columns"));
        }
    }
    view.extend(facts.sorted.map(str::to_owned));
    if facts.invisibles {
        view.push("¶ invisibles".to_owned());
    }

    let tail = match facts.tail {
        Tail::Held => String::new(),
        Tail::Complete => "Stream complete".to_owned(),
        // §12: the way back is named, because a paused tail whose pane says only "Paused" leaves
        // the reader to discover the key that resumes it.
        Tail::Paused => "‖ Paused — Ctrl+End".to_owned(),
        Tail::Following { lag: None } => "● Following".to_owned(),
        Tail::Following { lag: Some(lag) } => format!("● Following · {lag}"),
    };

    StatusPanes {
        message,
        position,
        find,
        filter,
        view: view.join(" · "),
        format: facts.format.unwrap_or_default().to_owned(),
        encoding: facts.encoding.unwrap_or_default().to_owned(),
        tail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> StatusFacts<'static> {
        StatusFacts {
            open: true,
            caret_row: Some(1_192),
            total_rows: 419_501,
            format: Some("Serilog"),
            encoding: Some("UTF-8"),
            ..StatusFacts::default()
        }
    }

    #[test]
    fn with_no_document_the_bar_says_so_and_every_other_pane_is_bare() {
        let panes = status_panes_of(StatusFacts::default());
        assert_eq!(panes.message, "Ready");
        assert_eq!(
            panes.parts()[1..],
            [""; 7],
            "a closed window must not leave stale facts on the bar: {panes:?}"
        );
    }

    #[test]
    fn the_position_pane_names_the_row_and_the_total_with_separators() {
        let panes = status_panes_of(open());
        assert_eq!(panes.position, "Line 1,192 of 419,501");
    }

    #[test]
    fn a_notice_takes_the_message_pane_and_displaces_nothing_else() {
        let panes = status_panes_of(StatusFacts {
            notice: Some("Exported 1,192 lines"),
            ..open()
        });
        assert_eq!(panes.message, "Exported 1,192 lines");
        assert_eq!(panes.position, "Line 1,192 of 419,501");
    }

    #[test]
    fn panes_that_have_nothing_to_say_are_empty_rather_than_absent() {
        let panes = status_panes_of(open());
        assert_eq!(panes.find, "");
        assert_eq!(panes.filter, "");
        assert_eq!(panes.view, "");
        assert_eq!(panes.tail, "");
    }

    /// **`SB_SETPARTS` requires non-decreasing right edges**, and the arithmetic that produces
    /// them used to live inside the `SendMessage` that consumes it, where nothing could test it.
    ///
    /// The case that breaks a naive running sum is a bar narrower than its own trailing panes — a
    /// small window carrying a long format name and a long filter. Left alone it hands comctl32 a
    /// part whose right edge is past the client rectangle and, once the final edge is forced to
    /// the bar's end, one that finishes *before* the part to its left.
    #[test]
    fn the_part_edges_never_go_backwards_however_narrow_the_bar() {
        for total in [0, 1, 40, 120, 400, 1_600] {
            for widths in [
                [0; StatusPanes::COUNT],
                [200, 90, 80, 140, 160, 110, 70, 60],
                [0, 300, 300, 300, 300, 300, 300, 300],
                [10, 0, 0, 0, 0, 0, 0, 5],
            ] {
                let edges = edges_of(total, widths);
                let last = StatusPanes::COUNT - 1;
                for i in 1..last {
                    assert!(
                        edges[i] >= edges[i - 1],
                        "edges went backwards at {i} for total {total}, widths {widths:?}: \
                         {edges:?}"
                    );
                    assert!(
                        edges[i] <= total,
                        "edge {i} is past the bar for total {total}, widths {widths:?}: {edges:?}"
                    );
                }
                assert_eq!(edges[last], TO_THE_EDGE, "the last part runs to the edge");
            }
        }
    }

    /// With room to spare the trailing panes get exactly what they measured, and the message pane
    /// keeps the rest — which is the whole point of the layout.
    #[test]
    fn the_message_pane_keeps_whatever_the_trailing_panes_do_not_need() {
        let widths = [0, 100, 0, 0, 0, 80, 60, 90];
        let edges = edges_of(1_000, widths);
        assert_eq!(edges[0], 1_000 - (100 + 80 + 60 + 90));
        assert_eq!(edges[1], edges[0] + 100);
        assert_eq!(edges[2], edges[1], "an empty pane takes no room");
    }

    #[test]
    fn a_find_reports_which_match_of_how_many() {
        let panes = status_panes_of(StatusFacts {
            find: Some(FindFacts {
                current: Some(3),
                total: 47,
                running: false,
            }),
            ..open()
        });
        assert_eq!(panes.find, "Match 3 of 47");
    }

    #[test]
    fn a_find_still_running_says_so_rather_than_claiming_a_total() {
        let panes = status_panes_of(StatusFacts {
            find: Some(FindFacts {
                current: None,
                total: 47,
                running: true,
            }),
            ..open()
        });
        assert_eq!(panes.find, "47 matches so far");
    }

    /// `LOKI.md` §6: a count over an answer Loki cut is a floor, and the bar may not round it up
    /// into a total. This is the same rule the title already keeps, on a new surface.
    #[test]
    fn a_count_over_a_cut_answer_is_a_floor_and_says_so() {
        let panes = status_panes_of(StatusFacts {
            find: Some(FindFacts {
                current: Some(3),
                total: 47,
                running: false,
            }),
            filter: Some((12, 1_192)),
            cut: true,
            ..open()
        });
        assert_eq!(panes.find, "Match 3 of at least 47");
        assert!(
            panes.filter.contains("at least"),
            "a filtered count over a cut answer is a floor too: {}",
            panes.filter
        );
    }

    #[test]
    fn a_filter_reports_what_it_kept_of_what_there_was() {
        let panes = status_panes_of(StatusFacts {
            filter: Some((12, 1_192)),
            ..open()
        });
        assert_eq!(panes.filter, "Filtered 12 of 1,192");
    }

    #[test]
    fn the_columns_pane_speaks_only_when_some_are_hidden() {
        let all = status_panes_of(StatusFacts {
            columns: Some((8, 8)),
            ..open()
        });
        assert_eq!(all.view, "", "nothing hidden, nothing to report");
        let some = status_panes_of(StatusFacts {
            columns: Some((3, 8)),
            ..open()
        });
        assert_eq!(some.view, "3 of 8 columns");
    }

    /// **A paused tail says so, and says how to resume.**
    ///
    /// The first cut of this had `following: bool` and rendered "not following" as an empty pane,
    /// which silently deleted `UI-DESIGN.md` §12's affordance — the behaviour that section calls
    /// "the single most-wanted behaviour in every tail tool". An empty pane is not an affordance.
    #[test]
    fn the_tail_pane_names_every_state_it_can_be_in() {
        let of = |tail| status_panes_of(StatusFacts { tail, ..open() }).tail;
        assert_eq!(of(Tail::Following { lag: None }), "● Following");
        assert_eq!(of(Tail::Paused), "‖ Paused — Ctrl+End");
        assert_eq!(of(Tail::Complete), "Stream complete");
        assert_eq!(
            of(Tail::Held),
            "",
            "a sort holds the view; it is not a tail state"
        );
    }

    /// **`UI-DESIGN.md` §4: "the lag is stated".**
    ///
    /// Without this a remote source that has gone quiet and one we are failing to keep up with
    /// look identical, and both look live. `StatusFacts` had no room for the lag at first, so
    /// wiring the bar up took Loki's only staleness indicator off the screen — against a standing
    /// instruction that Loki tailing is this project's priority.
    #[test]
    fn a_following_remote_source_states_how_far_behind_it_is() {
        let panes = status_panes_of(StatusFacts {
            tail: Tail::Following {
                lag: Some("current to 30s ago"),
            },
            ..open()
        });
        assert_eq!(panes.tail, "● Following · current to 30s ago");
    }

    /// The view pane carries three facts of one kind, and joins only the ones that apply.
    #[test]
    fn the_view_pane_gathers_hidden_columns_the_sort_and_revealed_invisibles() {
        let panes = status_panes_of(StatusFacts {
            columns: Some((3, 8)),
            sorted: Some("sorted by level"),
            invisibles: true,
            ..open()
        });
        assert_eq!(
            panes.view,
            "3 of 8 columns · sorted by level · ¶ invisibles"
        );

        let just_sorted = status_panes_of(StatusFacts {
            columns: Some((8, 8)),
            sorted: Some("sorted by level"),
            ..open()
        });
        assert_eq!(just_sorted.view, "sorted by level");
    }

    /// **Both, not whichever was checked first.**
    ///
    /// The composed sentence gave the notice and the rules warning separate slots and showed
    /// both. A first cut of this took `notice.or(rules)`, which let a transient notice hide a
    /// standing warning for as long as it showed — and the test that had proved the two combined
    /// was deleted with the sentence, so nothing would have caught it.
    #[test]
    fn a_transient_notice_never_hides_a_standing_warning() {
        let panes = status_panes_of(StatusFacts {
            notice: Some("format saved"),
            rules: Some("⚠ rules: amber"),
            contrast: true,
            cut: true,
            ..open()
        });
        for expected in [
            "format saved",
            "⚠ answers cut at the limit",
            "⚠ rules: amber",
            "⚑ High Contrast",
        ] {
            assert!(
                panes.message.contains(expected),
                "{expected:?} was dropped from {:?}",
                panes.message
            );
        }
        assert!(
            panes.message.find("format saved") < panes.message.find("⚠ rules"),
            "what just happened comes before what is standing: {:?}",
            panes.message
        );
    }

    /// E21: an export in flight is where the reader watches the file fill.
    #[test]
    fn an_export_in_flight_reaches_the_message_pane() {
        let panes = status_panes_of(StatusFacts {
            tee: Some("⇥ saving → out.log (12 lines)"),
            ..open()
        });
        assert_eq!(panes.message, "⇥ saving → out.log (12 lines)");
    }

    #[test]
    fn the_format_and_encoding_panes_carry_what_detection_settled() {
        let panes = status_panes_of(open());
        assert_eq!(panes.format, "Serilog");
        assert_eq!(panes.encoding, "UTF-8");
    }

    #[test]
    fn a_number_a_person_reads_is_grouped_in_threes() {
        assert_eq!(with_separators(0), "0");
        assert_eq!(with_separators(999), "999");
        assert_eq!(with_separators(1_000), "1,000");
        assert_eq!(with_separators(1_204_915), "1,204,915");
    }
}
