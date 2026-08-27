//! Random access to a row's text — the join between the index and the viewport.
//!
//! [`crate::index`] says which byte a line starts at, [`crate::lines`] turns bytes into `&str`, and
//! [`crate::view`] says which rows are on screen. Nothing joined them, so nothing could answer the
//! one question a painter asks: **what does row 4,812,003 say?**
//!
//! ## A viewport is consecutive, and that is the whole design
//!
//! [`offset_of_line`] costs an anchor lookup plus a forward scan over the lines between — `SPEC.md`
//! §5.3 puts that scan at 6.3 KB expected, and with the default stride of 64 it is bounded by 63
//! lines. Asking it once per visible row would pay that for **every** row of a screenful whose bytes
//! are contiguous on disk and already in the page cache.
//!
//! So [`Rows::fetch`] resolves the *first* row and decodes forward. Fifty rows cost one anchor
//! lookup and one sequential read, which is what the file layout was going to give us anyway.
//!
//! **Measured, on a 5,000-line corpus with the viewport at row 4,000: 69,088 bytes read for the
//! screenful, against 3,340,670 for the same fifty rows fetched one at a time — 48×.** The
//! per-row version is not *wrong*, which is why no other test here can catch it;
//! `a_screenful_costs_one_seek_rather_than_one_per_row` counts bytes rather than time so the claim
//! is deterministic and cannot flake under load.
//!
//! ## Why this holds a decoder rather than borrowing one
//!
//! [`LineDecoder`] is a state machine: it carries a partial trailing line and, for UTF-16, a
//! straddling code unit. Decoding from an arbitrary offset means **starting a fresh one** — a
//! decoder that has seen the preceding bytes would carry state that does not belong to this read.
//! That is why `fetch` builds one per call rather than keeping a long-lived decoder that would
//! silently mis-decode the first line after every seek.
//!
//! ## ⚠ What "not in memory" means here, and what §11.3 requires of it
//!
//! A row this cannot produce comes back as `None`, and [`Painter::lay_out`](crate::paint::Painter)
//! draws nothing for it. §11.3 requires exactly that — never block a frame on I/O — but a `None`
//! that means "past the end of the file" and a `None` that means "the read failed" are different
//! facts and this type does not conflate them: a failed read is recorded in [`Rows::last_error`] so
//! a caller can tell a short file from a broken one, and the frame still draws.

use crate::cell::{CellModel, ColumnAnchors};
use crate::encoding::Charset;
use crate::highlight::Span;
use crate::index::LineIndex;
use crate::indexer::{offset_of_line, ChunkReader};
use crate::lines::LineDecoder;
use crate::Result;

/// Bytes read per pass while filling a viewport.
///
/// §10.3 supports lines up to 32 KB, and a screenful is on the order of a hundred rows, so this is
/// sized to fetch an ordinary viewport in one or two reads without allocating for the pathological
/// case up front. It is a read size, not a limit: [`Rows::fetch`] loops until it has the rows it was
/// asked for or the file ends.
const READ_BYTES: usize = 128 * 1024;

/// The most bytes one `fetch` will read before giving up on the rows it has not yet produced.
///
/// **This is a frame budget, not a correctness bound.** §11.3 forbids blocking a frame, and a
/// viewport whose rows are pathologically long would otherwise read without limit while the window
/// is unresponsive. Rows past the cut come back as `None` and draw nothing this frame; the next
/// frame starts again with the same request and, because the pages are now warm, gets further.
const FETCH_BUDGET_BYTES: u64 = 8 * 1024 * 1024;

/// Bytes read per step while fetching **one scattered row** — see [`Rows::fetch_rows`].
///
/// Small, because a scattered fetch reads one row per read and a screenful of them is fifty reads:
/// at [`READ_BYTES`] that is 6 MB copied to show 6 KB of text. Almost every line fits in the first
/// step; the loop grows the read for the ones that do not.
const SCATTER_READ_BYTES: usize = 4 * 1024;

/// One box of the column header — `UI-DESIGN.md` §2.5.
///
/// Positions are in **cells**, not pixels, because that is what the grid and the hit-testing both
/// speak: `HGrid::x_of_column` turns a cell into an x that already accounts for the horizontal
/// scroll, and `Document::header_cell` turns a click back the other way. Keeping the boundary in
/// cells is what lets the divider drawn here and the resize target be the same edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeaderColumn {
    /// The column's title, as the user reads it. No padding and no sort marker — both are the
    /// painter's business now.
    pub title: String,
    /// The first cell of this column's box.
    pub start: usize,
    /// How many cells wide the box is, **including** the gap that separates it from the next. This
    /// is what makes the boxes tile: the next one starts at `start + cells`.
    pub cells: usize,
    /// How many cells the column's own data occupies, **excluding** that gap.
    ///
    /// **The divider goes at `start + content`, not at `start + cells`, and the difference is not
    /// cosmetic.** `Document::column_boundaries` puts the resize boundary at the end of the data —
    /// it adds the width, records the edge, and only then adds the gap — so a divider drawn at the
    /// box edge sits `GAP` cells to the right of the line the drag actually responds to. The line
    /// would be visibly beside its own grab target, which is worse than drawing nothing.
    pub content: usize,
    /// `Some(false)` ascending, `Some(true)` descending, `None` not sorted. §11.4 makes sorting a
    /// mode you leave following for, so the indicator matters: it is the only thing on screen that
    /// says why the rows are in this order.
    pub sort: Option<bool>,
}

/// Where a painter gets the text of a visible row, and the anchors that make placing it cheap.
///
/// **A trait rather than a closure because the two travel together.** The painter needs a row's text
/// *and* its [`ColumnAnchors`], and a `FnMut(u64) -> Option<String>` can supply neither by reference
/// — it allocated a `String` per row per frame and had nowhere to put the anchors. Implementing
/// `row_anchors` is optional: the default is the empty set, which every lookup accepts and which
/// only costs speed.
pub trait RowSource {
    /// The row's text, or `None` for a row not in memory — which draws nothing rather than blocking,
    /// per §11.3.
    fn row_text(&self, row: u64) -> Option<&str>;

    fn row_anchors(&self, _row: u64) -> &ColumnAnchors {
        ColumnAnchors::none_ref()
    }

    /// The **cell columns** selected on this row, if any.
    ///
    /// Columns rather than bytes, because `SPEC.md` §3.3 makes a column the unit a selection is
    /// expressed in and the painter is already placing clusters by column — converting to bytes here
    /// would mean converting straight back. `usize::MAX` as the end means "to the end of the line",
    /// which is what [`RowEnd::ToLineEnd`](crate::selection::RowEnd) means and what a stream
    /// selection produces for every row but its last.
    ///
    /// Defaulted to `None` so a source that knows nothing about selection — every test source, and
    /// `Rows` itself — is unaffected.
    fn row_selection(&self, _row: u64) -> Option<core::ops::Range<usize>> {
        None
    }

    /// The row a row-wise command would act on, if it is on screen.
    ///
    /// **This is the row `Ctrl+D`, `Define format from a line` and the bookmark steps use**, and it
    /// existed in the model for a long time with nothing on screen to say where it was. A single
    /// click places a caret — an empty selection — which selects nothing and therefore draws
    /// nothing, so the owner could click a line, choose a command that acts on "a line", and have
    /// no way to know which line that was. The point of this method is that the marker and the
    /// command read the *same* answer, rather than the screen offering a second opinion.
    ///
    /// In **view** rows, like everything else the painter is given.
    fn caret_row(&self) -> Option<u64> {
        None
    }

    /// The row's coloured runs, in **byte** offsets within it, sorted and non-overlapping.
    ///
    /// Bytes rather than the columns [`row_selection`](Self::row_selection) speaks in, because every
    /// producer of a span works in bytes: [`Highlighter::line`](crate::highlight::Highlighter::line)
    /// returns byte ranges and [`Match`](crate::search::Match) carries "byte offsets **within that
    /// line**, so a highlight does not need the file offset". The painter is walking clusters and
    /// already holds each one's byte offset, so matching there is a comparison rather than the
    /// column conversion doing it here would need — and a conversion the painter would immediately
    /// undo.
    ///
    /// **Filled rather than returned**, the same shape and for the same reason as
    /// `Highlighter::line`: the painter owns one `Vec` for the whole frame instead of allocating per
    /// row. `out` is cleared by the implementation.
    ///
    /// Defaulted to empty, so a source that knows nothing about colour draws exactly as before.
    fn row_spans(&self, _row: u64, out: &mut Vec<Span>) {
        out.clear();
    }

    /// The column header, when the source has columns — drawn in the band `View::header_px` reserves,
    /// in the same cells as the rows so it lines up with them. `None` means no band.
    ///
    /// Still the thing that decides **whether** there is a band. What goes *in* it is
    /// [`header_columns`](Self::header_columns) when that returns anything.
    fn header(&self) -> Option<&str> {
        None
    }

    /// The header as **boxes rather than one padded string** — `UI-DESIGN.md` §2.5.
    ///
    /// The padded-string form above lines each title up by counting monospace cells, which is why
    /// the header had to be drawn in the grid's face: the padding *is* the alignment. A Windows
    /// list header does not work that way. Each column is its own box, its label drawn at the box's
    /// left edge in the **UI font**, and the boxes are told apart by dividers rather than by
    /// spacing.
    ///
    /// So a source that has real columns returns them here and the painter places each label
    /// itself. Returning an empty list keeps the old single-string path, which is what a source
    /// with no layout wants.
    fn header_columns(&self) -> Vec<HeaderColumn> {
        Vec::new()
    }

    /// The number shown in the gutter for a view row — §6.4: "line numbers shown to the user are
    /// **physical** line numbers", so a filtered or collapsed view still says which line of the
    /// file this is. `None` draws nothing there. Default: the row plus one.
    fn row_number(&self, row: u64) -> Option<u64> {
        Some(row + 1)
    }

    /// A mark in the gutter for this row — a bookmark's colour. Default: none.
    fn row_mark(&self, _row: u64) -> Option<[f32; 4]> {
        None
    }

    /// `UI-DESIGN.md` §11.2's severity glyph — the **redundant non-colour channel** beside the line
    /// number: `■` fatal, `▲` error, `△` warning, `·` debug and trace, nothing for info or no
    /// severity. With its ink. Default: none.
    fn row_glyph(&self, _row: u64) -> Option<(char, [f32; 4])> {
        None
    }

    /// Draws the source's command bar into `View::chrome_px`'s band, with the painter's
    /// [`fill`](crate::paint::Painter::fill) and [`lay_out_at`](crate::paint::Painter::lay_out_at).
    /// Called once per frame before the rows, only when the band has height. Default: nothing.
    #[cfg(windows)]
    fn draw_chrome(&self, _painter: &mut crate::paint::Painter, _view: &crate::view::View) {}
}

impl RowSource for Rows {
    fn row_text(&self, row: u64) -> Option<&str> {
        self.line(row)
    }

    fn row_anchors(&self, row: u64) -> &ColumnAnchors {
        self.anchors(row)
    }
}

/// A window of decoded rows, and the reader and index they came from.
///
/// **The window is a list of rows, not a range**, since the filtered view (§7.3): a contiguous
/// viewport is the common case and fills `rows` as `first..first + n`; a hide-non-matching view asks
/// for the rows that survived, which are anywhere, through [`fetch_rows`](Rows::fetch_rows). Lookup
/// is a binary search over at most a screenful either way.
pub struct Rows {
    charset: Charset,
    /// The row number each entry of `lines` holds, ascending.
    rows: Vec<u64>,
    lines: Vec<String>,
    /// Column anchors, one per entry of `lines`. See [`ColumnAnchors`].
    anchors: Vec<ColumnAnchors>,
    /// What the last fetch was asked for — the rows, the index's line count, the anchor flag — so
    /// an identical request can be skipped.
    served: Option<(Vec<u64>, u64, bool)>,
    last_error: Option<String>,
}

impl Rows {
    pub fn new(charset: Charset) -> Self {
        Self {
            charset,
            rows: Vec::new(),
            lines: Vec::new(),
            anchors: Vec::new(),
            served: None,
            last_error: None,
        }
    }

    /// Fills the window with `count` rows starting at `first`, reading only what it must.
    ///
    /// Re-fetching an overlapping range still re-reads: this is a viewport buffer, not a cache with
    /// an eviction policy. Scrolling one row re-decodes the screenful, which is one sequential read
    /// of warm pages and is not what a frame's time goes on — the text pass is.
    pub fn fetch<R: ChunkReader + ?Sized>(
        &mut self,
        reader: &R,
        index: &LineIndex,
        first: u64,
        count: usize,
        anchored: bool,
    ) -> Result<()> {
        // **The same rows, again, are already here.** A horizontal scroll changes no row, and
        // `Document::lay_out` calls this every frame regardless — so without this a frame re-read
        // and re-decoded the whole viewport to arrive at exactly what it already held: 50 rows of
        // 19.4 KB is a megabyte of I/O and UTF-8 validation per frame, for nothing.
        //
        // **It is also what makes the anchors worth building.** They are built once here and used by
        // every frame until the row range moves, which is the difference between amortising one walk
        // per row and paying one per row per frame.
        //
        // The index's line count is part of the key: a file that grew has more rows to serve, and a
        // partial index (R5) that has since advanced can answer a request it previously could not.
        // What this deliberately does *not* detect is a file whose **contents** changed under a
        // stable line count — §5.5's copy-truncate. Following is M4 and will need to invalidate this
        // explicitly rather than rely on the key.
        // **`anchored` is part of the key, and that is what makes the anchors pay for themselves.**
        //
        // Building them is a full walk of every cluster in the row, and at column 0 that is *more*
        // work than the lookup it replaces: `byte_span`'s early exit stops after the ~150 clusters
        // the viewport shows, where a build visits all 19,400. Measured, that regressed a
        // column-0 page-down from 16 ms to 39 ms — a real cost, in the overwhelmingly common case,
        // to speed up a rarer one.
        //
        // So anchors are built only while the view is actually scrolled right. Putting the flag in
        // the key is what makes the transition work: scrolling off column 0 changes it, which forces
        // one refetch that builds them, and every frame after that is served from the cache.
        let wanted: Vec<u64> = (first..first.saturating_add(count as u64)).collect();
        if self.is_served(&wanted, index, anchored) {
            return Ok(());
        }
        // **Cleared here and only re-set on a clean read.** Recording the request up front would
        // cache a *failed* fetch, so a share that dropped for one frame would keep its truncated
        // viewport until something else moved.
        self.served = None;

        self.rows.clear();
        self.lines.clear();
        self.anchors.clear();
        self.last_error = None;
        if count == 0 {
            return Ok(());
        }

        let start = match offset_of_line(reader, self.charset, index, first) {
            Ok(Some(offset)) => offset,
            // Past what has been indexed — a partial index (R5) must be able to say so, and a
            // viewport scrolled past the end of a short file is the ordinary case.
            Ok(None) => return Ok(()),
            Err(e) => {
                self.last_error = Some(e.0);
                return Ok(());
            }
        };

        // A fresh decoder, because this read begins at an arbitrary offset. See the module note.
        let mut decoder = LineDecoder::new(self.charset);
        let mut buf = vec![0u8; READ_BYTES];
        let mut at = start;
        let mut read_total = 0u64;

        while self.lines.len() < count && read_total < FETCH_BUDGET_BYTES {
            let read = match reader.read_at(at, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    // **Not a reason to lose the rows already decoded.** A read failing partway
                    // through a viewport still leaves the earlier rows true, and §11.3's "draws
                    // nothing rather than blocking" is about the rows it could not get, not the
                    // ones it did.
                    self.last_error = Some(e.0);
                    break;
                }
            };
            at += read as u64;
            read_total += read as u64;

            let lines = &mut self.lines;
            decoder.push(&buf[..read], |line| {
                if lines.len() < count {
                    lines.push(line.to_owned());
                }
            });
        }

        // The final line of a file with no trailing terminator is a real line and §5.6's
        // never-discard-content rule reaches it: without this it would be held in the decoder and
        // never drawn, so a file not ending in a newline would appear one line short.
        if self.lines.len() < count && read_total < FETCH_BUDGET_BYTES {
            let lines = &mut self.lines;
            decoder.finish(|line| {
                if lines.len() < count {
                    lines.push(line.to_owned());
                }
            });
        }

        self.rows
            .extend((0..self.lines.len() as u64).map(|i| first + i));
        self.finish_fetch(wanted, index, anchored);
        Ok(())
    }

    /// Fills the window with exactly `rows` — **any rows, in ascending order** — one positioned read
    /// each. This is what a hide-non-matching view (§7.3) asks for: the rows that survived a filter
    /// are anywhere, and a contiguous window would fetch a screenful to show one line of it.
    ///
    /// A row past the index, or one whose read fails, is left out; the others are served, per
    /// §11.3. Cached by the same key as [`fetch`](Self::fetch), so a frame that shows the same
    /// filtered rows again reads nothing.
    pub fn fetch_rows<R: ChunkReader + ?Sized>(
        &mut self,
        reader: &R,
        index: &LineIndex,
        rows: &[u64],
        anchored: bool,
    ) -> Result<()> {
        debug_assert!(rows.windows(2).all(|w| w[0] < w[1]), "ascending, distinct");
        let wanted = rows.to_vec();
        if self.is_served(&wanted, index, anchored) {
            return Ok(());
        }
        self.served = None;
        self.rows.clear();
        self.lines.clear();
        self.anchors.clear();
        self.last_error = None;

        let mut buf = vec![0u8; SCATTER_READ_BYTES];
        let mut read_total = 0u64;
        for &row in rows {
            if read_total >= FETCH_BUDGET_BYTES {
                break;
            }
            let start = match offset_of_line(reader, self.charset, index, row) {
                Ok(Some(offset)) => offset,
                Ok(None) => continue,
                Err(e) => {
                    self.last_error = Some(e.0);
                    break;
                }
            };
            // One line: read in small steps until the decoder gives one up, or the file ends.
            let mut decoder = LineDecoder::new(self.charset);
            let mut at = start;
            let mut line: Option<String> = None;
            let mut this_row = 0usize;
            while line.is_none() && this_row < READ_BYTES {
                let read = match reader.read_at(at, &mut buf) {
                    Ok(0) => {
                        decoder.finish(|text| {
                            line.get_or_insert_with(|| text.to_owned());
                        });
                        break;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        self.last_error = Some(e.0);
                        break;
                    }
                };
                at += read as u64;
                this_row += read;
                read_total += read as u64;
                decoder.push(&buf[..read], |text| {
                    line.get_or_insert_with(|| text.to_owned());
                });
            }
            if self.last_error.is_some() {
                break;
            }
            if let Some(text) = line {
                self.rows.push(row);
                self.lines.push(text);
            }
        }
        self.finish_fetch(wanted, index, anchored);
        Ok(())
    }

    fn is_served(&self, wanted: &[u64], index: &LineIndex, anchored: bool) -> bool {
        matches!(&self.served, Some((rows, count, a)) if rows == wanted && *count == index.line_count() && *a == anchored)
    }

    /// Builds the anchors and records the request as served, for both fetch paths.
    fn finish_fetch(&mut self, wanted: Vec<u64>, index: &LineIndex, anchored: bool) {
        // **One walk per row, here, instead of one per row per frame** — but only when the caller
        // says the view is scrolled right, per the note on the cache key above.
        if anchored {
            let model = CellModel::new();
            self.anchors
                .extend(self.lines.iter().map(|l| ColumnAnchors::build(&model, l)));
        }
        if self.last_error.is_none() {
            self.served = Some((wanted, index.line_count(), anchored));
        }
    }

    /// A row's column anchors, or an empty set — which every lookup accepts.
    pub fn anchors(&self, row: u64) -> &ColumnAnchors {
        let found = self.slot(row).and_then(|i| self.anchors.get(i));
        match found {
            Some(a) => a,
            None => ColumnAnchors::none_ref(),
        }
    }

    /// The text of an absolute row number, or `None` if this window does not hold it.
    ///
    /// **This is what [`Renderer::paint_rows`](crate::Renderer) wants**, and the signature is
    /// deliberately the one a painter can call per row without the painter knowing what a byte
    /// offset is.
    pub fn line(&self, row: u64) -> Option<&str> {
        self.lines.get(self.slot(row)?).map(String::as_str)
    }

    /// Where `row` sits in the window, if it does. A binary search over a screenful.
    fn slot(&self, row: u64) -> Option<usize> {
        self.rows.binary_search(&row).ok()
    }

    /// The lowest row the window holds, or 0 for an empty one.
    pub fn first(&self) -> u64 {
        self.rows.first().copied().unwrap_or(0)
    }

    /// How many rows the window actually holds — fewer than asked for at end of file, after a read
    /// error, or when [`FETCH_BUDGET_BYTES`] ran out.
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// **The read error from the last [`fetch`](Self::fetch), if there was one.**
    ///
    /// A short window and a broken read are indistinguishable from [`len`](Self::len) alone, and
    /// they mean very different things — one is the end of a file, the other is a disk or a network
    /// share that has stopped answering. The frame draws either way; this is how a caller tells
    /// them apart.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::LineIndex;
    use crate::indexer::{build_index, IndexOptions};

    const UTF8: Charset = Charset::UTF_8;

    fn indexed(text: &[u8]) -> LineIndex {
        build_index(text, UTF8, 0, text.len() as u64, &IndexOptions::default()).expect("index")
    }

    fn corpus(lines: usize) -> Vec<u8> {
        let mut out = String::new();
        for i in 0..lines {
            out.push_str(&format!("line {i} — the quick brown fox\n"));
        }
        out.into_bytes()
    }

    /// The filtered view (§7.3) asks for rows that are anywhere; each comes back by its own number,
    /// nothing in between is read into the window, and the last line without a terminator is a row.
    #[test]
    fn scattered_rows_are_served_by_number_and_nothing_between_them() {
        let mut text = corpus(5_000);
        text.extend_from_slice(b"last line, no newline");
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        rows.fetch_rows(&text[..], &index, &[7, 4_000, 4_999, 5_000], false)
            .expect("fetch_rows");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows.line(7), Some("line 7 — the quick brown fox"));
        assert_eq!(rows.line(4_000), Some("line 4000 — the quick brown fox"));
        assert_eq!(rows.line(4_999), Some("line 4999 — the quick brown fox"));
        assert_eq!(rows.line(5_000), Some("last line, no newline"));
        assert_eq!(rows.line(8), None, "not asked for, not held");
        assert_eq!(rows.line(5_001), None, "past the file");

        // The same request again reads nothing: the served key covers the scattered path too.
        struct Counting<'a>(&'a [u8], std::sync::atomic::AtomicUsize);
        impl ChunkReader for Counting<'_> {
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
                self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.0.read_at(offset, buf)
            }
        }
        let counting = Counting(&text, std::sync::atomic::AtomicUsize::new(0));
        rows.fetch_rows(&counting, &index, &[7, 4_000, 4_999, 5_000], false)
            .expect("again");
        assert_eq!(
            counting.1.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "an identical request is served from the window"
        );

        // A different list is a different window, and a contiguous fetch after it works as before.
        rows.fetch_rows(&counting, &index, &[1, 2], false)
            .expect("other");
        assert!(counting.1.load(std::sync::atomic::Ordering::Relaxed) > 0);
        assert_eq!(rows.line(1), Some("line 1 — the quick brown fox"));
        assert_eq!(rows.line(7), None);
        rows.fetch(&text[..], &index, 10, 3, false).expect("fetch");
        assert_eq!(rows.line(11), Some("line 11 — the quick brown fox"));
        assert_eq!(rows.line(1), None);
    }

    /// A scattered fetch of a row far longer than one read step still comes back whole.
    #[test]
    fn a_long_scattered_row_is_read_in_steps_until_it_ends() {
        let mut text = corpus(10);
        let long = "x".repeat(20 * 1024);
        text.extend_from_slice(long.as_bytes());
        text.extend_from_slice(
            b"
after
",
        );
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);
        rows.fetch_rows(&text[..], &index, &[10, 11], false)
            .expect("fetch_rows");
        assert_eq!(rows.line(10).map(str::len), Some(20 * 1024));
        assert_eq!(rows.line(11), Some("after"));
    }

    #[test]
    fn a_viewport_in_the_middle_of_a_file_reads_the_rows_it_asked_for() {
        let text = corpus(5_000);
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        rows.fetch(&text[..], &index, 4_000, 50, false)
            .expect("fetch");

        assert_eq!(rows.len(), 50);
        assert_eq!(rows.line(4_000), Some("line 4000 — the quick brown fox"));
        assert_eq!(rows.line(4_049), Some("line 4049 — the quick brown fox"));
        // Rows outside the window are not this window's business.
        assert_eq!(rows.line(3_999), None);
        assert_eq!(rows.line(4_050), None);
        assert!(rows.last_error().is_none());
    }

    /// Every row must be reachable, not just the ones that happen to sit on an anchor. With the
    /// default stride of 64, a row at `anchor + 63` is the deepest forward scan there is.
    #[test]
    fn every_row_resolves_regardless_of_where_the_anchors_fell() {
        let text = corpus(600);
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        for row in 0..600u64 {
            rows.fetch(&text[..], &index, row, 1, false).expect("fetch");
            assert_eq!(
                rows.line(row),
                Some(format!("line {row} — the quick brown fox").as_str()),
                "row {row} came back wrong"
            );
        }
    }

    /// **The module's one design claim, measured in bytes rather than in time.**
    ///
    /// "Resolve the first row and decode forward" is only worth writing down if the obvious
    /// alternative — `offset_of_line` per visible row — is genuinely worse, and a per-row
    /// implementation would be *correct*, so no other test here can tell the difference. Counting
    /// bytes read makes the claim checkable without a clock: it is deterministic, so it cannot flake
    /// under load the way a duration does, and it is the quantity the design is actually about.
    #[test]
    fn a_screenful_costs_one_seek_rather_than_one_per_row() {
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Counting {
            text: Vec<u8>,
            bytes: AtomicU64,
        }
        impl ChunkReader for Counting {
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
                let n = ChunkReader::read_at(&self.text[..], offset, buf)?;
                self.bytes.fetch_add(n as u64, Ordering::Relaxed);
                Ok(n)
            }
        }

        let text = corpus(5_000);
        let index = indexed(&text);
        let reader = Counting {
            text: text.clone(),
            bytes: AtomicU64::new(0),
        };
        let mut rows = Rows::new(UTF8);

        // One fetch of a screenful.
        rows.fetch(&reader, &index, 4_000, 50, false)
            .expect("batch fetch");
        assert_eq!(rows.len(), 50);
        let batched = reader.bytes.swap(0, Ordering::Relaxed);

        // The same fifty rows, one at a time — what the per-row alternative would do.
        for row in 4_000..4_050u64 {
            rows.fetch(&reader, &index, row, 1, false)
                .expect("per-row fetch");
        }
        let per_row = reader.bytes.swap(0, Ordering::Relaxed);

        assert!(
            per_row > batched * 10,
            "a screenful read {batched} bytes and fifty single rows read {per_row} — \
             the forward decode is no longer saving anything, so either the design was lost \
             or the read sizes changed under it"
        );
    }

    /// **Asking for the same rows again reads nothing — and asking for different ones does.**
    ///
    /// `Document::lay_out` calls `fetch` every frame, and a horizontal scroll changes no row, so
    /// without this a frame re-read and re-decoded a viewport it already held. It is a cache, which
    /// means the interesting assertions are the ones about it *not* being used: a moved range, a
    /// changed row count, and a read that failed must all fetch again.
    #[test]
    fn the_same_rows_are_not_read_twice_and_different_ones_are() {
        use std::sync::atomic::{AtomicU64, Ordering};

        struct Counting {
            text: Vec<u8>,
            bytes: AtomicU64,
        }
        impl ChunkReader for Counting {
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
                let n = ChunkReader::read_at(&self.text[..], offset, buf)?;
                self.bytes.fetch_add(n as u64, Ordering::Relaxed);
                Ok(n)
            }
        }

        let text = corpus(2_000);
        let index = indexed(&text);
        let reader = Counting {
            text: text.clone(),
            bytes: AtomicU64::new(0),
        };
        let mut rows = Rows::new(UTF8);

        rows.fetch(&reader, &index, 500, 40, false).expect("first");
        let first = reader.bytes.swap(0, Ordering::Relaxed);
        assert!(first > 0);
        assert_eq!(rows.len(), 40);

        // The same window, again: nothing read, and the rows are still there.
        rows.fetch(&reader, &index, 500, 40, false).expect("repeat");
        assert_eq!(
            reader.bytes.swap(0, Ordering::Relaxed),
            0,
            "re-read the same rows"
        );
        assert_eq!(rows.line(500), Some("line 500 — the quick brown fox"));
        assert_eq!(rows.len(), 40);

        // One row down is a different window.
        rows.fetch(&reader, &index, 501, 40, false).expect("moved");
        assert!(
            reader.bytes.swap(0, Ordering::Relaxed) > 0,
            "a moved window was served stale"
        );
        assert_eq!(rows.line(501), Some("line 501 — the quick brown fox"));

        // A different count is a different window too.
        rows.fetch(&reader, &index, 501, 41, false)
            .expect("resized");
        assert!(
            reader.bytes.swap(0, Ordering::Relaxed) > 0,
            "a resized window was served stale"
        );

        // **A grown file must not be served from the cache.** The index's line count is in the key
        // precisely so that a partial index that has advanced, or a file being followed, refetches.
        let grown = indexed(&corpus(2_500));
        rows.fetch(&reader, &grown, 501, 41, false).expect("grown");
        assert!(
            reader.bytes.swap(0, Ordering::Relaxed) > 0,
            "a changed line count was served from the cache"
        );
    }

    /// A failed read must not be cached, or one bad frame would stick until something else moved.
    #[test]
    fn a_failed_fetch_is_retried_rather_than_remembered() {
        struct Flaky {
            text: Vec<u8>,
            fail: std::cell::Cell<bool>,
        }
        // `ChunkReader` requires `Sync`; the test is single-threaded and never shares this.
        unsafe impl Sync for Flaky {}
        impl ChunkReader for Flaky {
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
                if self.fail.get() {
                    return Err(crate::Error("the share went away".into()));
                }
                ChunkReader::read_at(&self.text[..], offset, buf)
            }
        }

        let text = corpus(500);
        let index = indexed(&text);
        let reader = Flaky {
            text: text.clone(),
            fail: std::cell::Cell::new(true),
        };
        let mut rows = Rows::new(UTF8);

        rows.fetch(&reader, &index, 10, 20, false).expect("fetch");
        assert!(rows.last_error().is_some());
        assert!(rows.is_empty());

        // The share comes back. The identical request must be *served*, not skipped.
        reader.fail.set(false);
        rows.fetch(&reader, &index, 10, 20, false).expect("retry");
        assert!(rows.last_error().is_none(), "the failure was remembered");
        assert_eq!(rows.line(10), Some("line 10 — the quick brown fox"));
    }

    /// §5.6 — the last line of a file with no trailing newline is content, and content is never
    /// discarded silently. Without `decoder.finish` it sits in the decoder and the file looks one
    /// line short.
    #[test]
    fn a_file_with_no_trailing_newline_still_shows_its_last_line() {
        let text = b"alpha\nbeta\ngamma".to_vec();
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        rows.fetch(&text[..], &index, 0, 10, false).expect("fetch");

        assert_eq!(rows.len(), 3, "the unterminated last line was dropped");
        assert_eq!(rows.line(2), Some("gamma"));
    }

    /// A viewport scrolled past the end of a short file is ordinary, not an error: it draws the
    /// rows that exist and nothing for the rest.
    #[test]
    fn a_viewport_past_the_end_draws_what_there_is() {
        let text = corpus(10);
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        rows.fetch(&text[..], &index, 5, 50, false).expect("fetch");
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.line(9), Some("line 9 — the quick brown fox"));
        assert_eq!(rows.line(10), None);
        assert!(rows.last_error().is_none(), "end of file is not an error");

        // And entirely past the end is empty rather than a failure.
        rows.fetch(&text[..], &index, 500, 50, false)
            .expect("fetch");
        assert!(rows.is_empty());
        assert!(rows.last_error().is_none());
    }

    /// A reader that fails partway must not cost the rows already decoded, and must not look like
    /// end of file — §11.3 draws what it has, and the caller can still tell the two apart.
    #[test]
    fn a_read_that_fails_partway_keeps_what_it_had_and_says_so() {
        struct FailsAfter {
            text: Vec<u8>,
            after: u64,
        }
        impl ChunkReader for FailsAfter {
            fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
                if offset >= self.after {
                    return Err(crate::Error("the share went away".into()));
                }
                // One short read at a time, so the failure lands mid-viewport rather than before
                // a single big read that would have covered everything.
                let end = (offset + 64).min(self.after);
                let n = ChunkReader::read_at(
                    &self.text[..],
                    offset,
                    &mut buf[..(end - offset) as usize],
                )?;
                Ok(n)
            }
        }

        let text = corpus(200);
        let index = indexed(&text);
        let reader = FailsAfter {
            text: text.clone(),
            after: 256,
        };
        let mut rows = Rows::new(UTF8);

        rows.fetch(&reader, &index, 0, 50, false)
            .expect("fetch itself does not fail");

        assert!(
            !rows.is_empty(),
            "the rows decoded before the failure were thrown away"
        );
        assert!(rows.len() < 50, "the fixture was meant to fail partway");
        assert_eq!(rows.line(0), Some("line 0 — the quick brown fox"));
        assert_eq!(
            rows.last_error(),
            Some("the share went away"),
            "a broken read is indistinguishable from a short file without this"
        );
    }
}
