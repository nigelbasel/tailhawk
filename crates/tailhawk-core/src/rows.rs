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

use crate::encoding::Charset;
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

/// A window of decoded rows, and the reader and index they came from.
pub struct Rows {
    charset: Charset,
    /// The row number `lines[0]` holds.
    first: u64,
    lines: Vec<String>,
    last_error: Option<String>,
}

impl Rows {
    pub fn new(charset: Charset) -> Self {
        Self {
            charset,
            first: 0,
            lines: Vec::new(),
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
    ) -> Result<()> {
        self.first = first;
        self.lines.clear();
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
        Ok(())
    }

    /// The text of an absolute row number, or `None` if this window does not hold it.
    ///
    /// **This is what [`Renderer::paint_rows`](crate::Renderer) wants**, and the signature is
    /// deliberately the one a painter can call per row without the painter knowing what a byte
    /// offset is.
    pub fn line(&self, row: u64) -> Option<&str> {
        let i = row.checked_sub(self.first)?;
        self.lines.get(usize::try_from(i).ok()?).map(String::as_str)
    }

    /// The row the window starts at.
    pub fn first(&self) -> u64 {
        self.first
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

    #[test]
    fn a_viewport_in_the_middle_of_a_file_reads_the_rows_it_asked_for() {
        let text = corpus(5_000);
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        rows.fetch(&text[..], &index, 4_000, 50).expect("fetch");

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
            rows.fetch(&text[..], &index, row, 1).expect("fetch");
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
        rows.fetch(&reader, &index, 4_000, 50).expect("batch fetch");
        assert_eq!(rows.len(), 50);
        let batched = reader.bytes.swap(0, Ordering::Relaxed);

        // The same fifty rows, one at a time — what the per-row alternative would do.
        for row in 4_000..4_050u64 {
            rows.fetch(&reader, &index, row, 1).expect("per-row fetch");
        }
        let per_row = reader.bytes.swap(0, Ordering::Relaxed);

        assert!(
            per_row > batched * 10,
            "a screenful read {batched} bytes and fifty single rows read {per_row} — \
             the forward decode is no longer saving anything, so either the design was lost \
             or the read sizes changed under it"
        );
    }

    /// §5.6 — the last line of a file with no trailing newline is content, and content is never
    /// discarded silently. Without `decoder.finish` it sits in the decoder and the file looks one
    /// line short.
    #[test]
    fn a_file_with_no_trailing_newline_still_shows_its_last_line() {
        let text = b"alpha\nbeta\ngamma".to_vec();
        let index = indexed(&text);
        let mut rows = Rows::new(UTF8);

        rows.fetch(&text[..], &index, 0, 10).expect("fetch");

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

        rows.fetch(&text[..], &index, 5, 50).expect("fetch");
        assert_eq!(rows.len(), 5);
        assert_eq!(rows.line(9), Some("line 9 — the quick brown fox"));
        assert_eq!(rows.line(10), None);
        assert!(rows.last_error().is_none(), "end of file is not an error");

        // And entirely past the end is empty rather than a failure.
        rows.fetch(&text[..], &index, 500, 50).expect("fetch");
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

        rows.fetch(&reader, &index, 0, 50)
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
