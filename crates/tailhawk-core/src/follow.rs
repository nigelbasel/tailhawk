//! Following a growing file — `SPEC.md` §5.5 and §11.3, the growth half.
//!
//! [`crate::indexer`] builds the index a file has when it is opened. This is the one it grows
//! afterwards: poll the length, scan only what is new, append the lines found.
//!
//! ## The handle is polled, not the directory
//!
//! §5.5 is explicit that rotation detection keyed on the **path** is the bug, and the same reasoning
//! applies to watching for growth: a directory notification says a name changed, which is not the
//! question. The open handle's length answers "is there more of *this* file", which is. It also
//! costs one `GetFileSizeEx` per tick against a file already open, so there is nothing to save by
//! being cleverer.
//!
//! **This module handles growth only.** A file that *shrinks* is truncation or rotation, and that is
//! a different component with its own `CLEANROOM.md` row — [`Poll::Shrank`] reports it and stops
//! rather than guessing, because guessing wrong loses bytes.
//!
//! ## The phantom line is the whole subtlety
//!
//! `build_index` ends with:
//!
//! ```text
//! if final_start.is_some() && final_start == data_end { index.pop_line(); }
//! ```
//!
//! — a terminator as the file's last bytes opens a line start at end of data, and that is not a line
//! until something follows it. **Following is precisely the case where something follows it.** So
//! the popped line has to come back the instant a byte arrives, or every append is one line short
//! for ever after; and the new scan must then re-apply the same rule at its own end. That is what
//! [`Follow::pending_line`] carries between ticks, and
//! `a_line_appended_after_a_trailing_newline_is_not_lost` is the test that fails without it.

use crate::encoding::Charset;
use crate::index::{LineIndex, LineScanner};
use crate::indexer::ChunkReader;
use crate::Result;

/// Bytes scanned in one tick before returning to the message loop.
///
/// **§11.3's per-tick budget, and the reason this returns rather than looping to the end.** A writer
/// producing faster than the frame rate would otherwise keep the scan running for ever and the
/// window would stop answering. 4 MB is roughly 50 ms of scanning on the machine this was built on,
/// against a 16.67 ms frame — deliberately more than one frame's worth, because the alternative is
/// falling behind a fast writer, and §11.3's requirement is that the UI stays responsive, not that
/// every tick is short.
pub const FOLLOW_BUDGET_BYTES: u64 = 4 * 1024 * 1024;

/// Read size within a tick. One comfortable read, matching `indexer`'s own.
const READ_BYTES: usize = 256 * 1024;

/// What one [`Follow::poll`] found.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Poll {
    /// The file is the length it was. Nothing was scanned.
    Unchanged,
    /// Lines were appended. `more` is true when the budget ran out before the end of the file, so
    /// the caller should poll again rather than wait for the next change.
    Grew { lines: u64, more: bool },
    /// **The file is shorter than it was.** Truncation or rotation, §5.5 — not this module's
    /// business, and reported rather than guessed at because guessing loses bytes.
    Shrank { was: u64, now: u64 },
}

/// Tracks how far a file has been indexed, so the next tick can resume exactly where it stopped.
#[derive(Clone, Debug)]
pub struct Follow {
    charset: Charset,
    /// Absolute offset one past the last byte scanned.
    scanned_to: u64,
    /// **A line begins at `scanned_to` as soon as any byte arrives.**
    ///
    /// True when the last byte scanned completed a terminator. See the module note: `build_index`
    /// pops that line because it does not exist yet, and following is where it comes into being.
    pending_line: bool,
}

impl Follow {
    /// Starts following a file already indexed to `scanned_to`.
    ///
    /// `ended_on_terminator` is what `build_index` learned and threw away — pass true when the
    /// indexed region ends exactly on a line terminator. [`Follow::after_build`] works it out
    /// instead, and is what callers should normally use.
    pub fn new(charset: Charset, scanned_to: u64, ended_on_terminator: bool) -> Self {
        Self {
            charset,
            scanned_to,
            pending_line: ended_on_terminator,
        }
    }

    /// Starts following from a freshly built index.
    ///
    /// The extent knows whether the indexed region ends on a terminator —
    /// [`Extent::ends_on_terminator`] — which is the question `pending_line` asks, so nothing has to
    /// be threaded through `build_index` to find it out.
    ///
    /// **An empty file is the other case, and it is not the same one.** `scan_chunk` seeds the
    /// file's opening line through its `owns_first_line` flag, and a file that was empty when it was
    /// indexed never reached that code — so line 0 is unclaimed and the first byte to arrive starts
    /// it. Without this, following a file from empty loses its first line permanently and every
    /// count afterwards is one short. `following_byte_by_byte_agrees_with_indexing_the_whole_file`
    /// is what caught it; a file with content at open time never exercises the path.
    pub fn after_build(charset: Charset, index: &LineIndex, indexed_to: u64) -> Self {
        let opening_line_unclaimed = index.line_count() == 0;
        Self::new(
            charset,
            indexed_to,
            index.extent().ends_on_terminator() || opening_line_unclaimed,
        )
    }

    pub fn scanned_to(&self) -> u64 {
        self.scanned_to
    }

    /// Scans whatever has been appended, up to [`FOLLOW_BUDGET_BYTES`], appending lines to `index`.
    ///
    /// `len` is the file's length now — the caller supplies it because the caller is the one holding
    /// a reason to have asked (a timer tick, a change notification), and asking twice would race.
    pub fn poll<R: ChunkReader + ?Sized>(
        &mut self,
        reader: &R,
        index: &mut LineIndex,
        len: u64,
    ) -> Result<Poll> {
        if len < self.scanned_to {
            return Ok(Poll::Shrank {
                was: self.scanned_to,
                now: len,
            });
        }
        if len == self.scanned_to {
            return Ok(Poll::Unchanged);
        }

        let stop = len.min(self.scanned_to + FOLLOW_BUDGET_BYTES);
        let mut scanner = LineScanner::new(self.charset, self.scanned_to);
        let mut buf = vec![0u8; READ_BYTES];
        let mut at = self.scanned_to;
        let mut appended = 0u64;
        let mut last_start = None;
        let mut seeded = false;

        while at < stop {
            let want = usize::try_from(stop - at)
                .unwrap_or(buf.len())
                .min(buf.len());
            let read = reader.read_at(at, &mut buf[..want])?;
            if read == 0 {
                // The file shrank under the read. Stop rather than spin; the next poll sees the
                // shorter length and reports `Shrank`.
                break;
            }
            // **The pending line becomes real here, not before the read.** A file that reported a
            // longer length and then gave back nothing has not gained a line, and seeding up front
            // would invent one — the same reasoning `scan_chunk` uses for the file's opening line.
            if self.pending_line && !seeded {
                index.push_line(self.scanned_to);
                last_start = Some(self.scanned_to);
                appended += 1;
                seeded = true;
                self.pending_line = false;
            }
            scanner.push(&buf[..read], |offset| {
                index.push_line(offset);
                last_start = Some(offset);
                appended += 1;
            });
            at += read as u64;
        }

        self.scanned_to = at;

        // The same rule `build_index` ends with, applied to this run: a terminator as the last bytes
        // opens a line start that is not yet a line. Popped now and remembered, so the next tick
        // puts it back if more arrives.
        if last_start == Some(self.scanned_to) {
            index.pop_line();
            appended -= 1;
            self.pending_line = true;
        }

        index.set_extent(index.extent().merge(scanner.extent()));
        Ok(Poll::Grew {
            lines: appended,
            more: self.scanned_to < len,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::{build_index, offset_of_line, IndexOptions};

    const UTF8: Charset = Charset::UTF_8;

    /// Builds an index over `initial`, then follows into `appended`, returning the index and the
    /// whole file — so every test asserts against a single source of truth.
    fn follow(initial: &str, appended: &str) -> (LineIndex, Vec<u8>) {
        let whole = format!("{initial}{appended}").into_bytes();
        let start = initial.as_bytes();
        let mut index = build_index(start, UTF8, 0, start.len() as u64, &IndexOptions::default())
            .expect("build");
        let mut f = Follow::after_build(UTF8, &index, start.len() as u64);
        let mut guard = 0;
        while let Poll::Grew { more: true, .. } = f
            .poll(&whole[..], &mut index, whole.len() as u64)
            .expect("poll")
        {
            guard += 1;
            assert!(guard < 1000, "poll never reached the end");
        }
        (index, whole)
    }

    /// The whole file indexed in one go is the answer every follow must agree with.
    fn oracle(text: &[u8]) -> LineIndex {
        build_index(text, UTF8, 0, text.len() as u64, &IndexOptions::default()).expect("oracle")
    }

    /// **The phantom line, and the reason this module exists.**
    ///
    /// `build_index` pops the line a trailing terminator opens, because it is not a line until
    /// something follows it. Following is exactly when something does. Without `pending_line` the
    /// appended line is never counted and the file is one row short from then on.
    #[test]
    fn a_line_appended_after_a_trailing_newline_is_not_lost() {
        let (index, whole) = follow("alpha\nbeta\n", "gamma\n");
        assert_eq!(index.line_count(), 3, "the appended line was lost");
        assert_eq!(index.line_count(), oracle(&whole).line_count());

        let at = offset_of_line(&whole[..], UTF8, &index, 2)
            .expect("resolve")
            .expect("line 2");
        assert_eq!(&whole[at as usize..at as usize + 5], b"gamma");
    }

    /// Appending to a file that ended **mid-line** continues that line rather than starting one.
    #[test]
    fn appending_to_an_unterminated_last_line_continues_it() {
        let (index, whole) = follow("alpha\nbet", "a\ngamma\n");
        assert_eq!(index.line_count(), 3);
        assert_eq!(index.line_count(), oracle(&whole).line_count());

        let at = offset_of_line(&whole[..], UTF8, &index, 1)
            .expect("resolve")
            .expect("line 1");
        assert_eq!(&whole[at as usize..at as usize + 4], b"beta");
    }

    /// **Following in many small steps must equal indexing the whole file once.**
    ///
    /// This is the differential the module is really claiming: a writer appends a byte at a time,
    /// and every line boundary still lands where a single scan would have put it. It runs the
    /// awkward shapes together — a terminator arriving on its own tick, an empty line, a file that
    /// never ends with a terminator.
    #[test]
    fn following_byte_by_byte_agrees_with_indexing_the_whole_file() {
        for text in [
            "alpha\nbeta\ngamma\n",
            "alpha\nbeta\ngamma",
            "\n\n\n",
            "a\n\nb\n",
            "no terminators at all",
            "",
            "\n",
        ] {
            let bytes = text.as_bytes();
            let mut index =
                build_index(&bytes[..0], UTF8, 0, 0, &IndexOptions::default()).expect("empty");
            let mut f = Follow::after_build(UTF8, &index, 0);

            for end in 1..=bytes.len() {
                f.poll(&bytes[..end], &mut index, end as u64).expect("poll");
            }

            assert_eq!(
                index.line_count(),
                oracle(bytes).line_count(),
                "byte-by-byte follow disagreed on {text:?}"
            );
            // Every line must also resolve to the same byte, not merely be counted.
            for line in 0..index.line_count() {
                assert_eq!(
                    offset_of_line(bytes, UTF8, &index, line).expect("follow"),
                    offset_of_line(bytes, UTF8, &oracle(bytes), line).expect("oracle"),
                    "line {line} of {text:?} resolved differently"
                );
            }
        }
    }

    /// A file that has not moved costs nothing, and one that shrank is reported rather than guessed.
    #[test]
    fn an_unchanged_file_is_untouched_and_a_shorter_one_is_reported() {
        let text = b"alpha\nbeta\n";
        let mut index = build_index(
            &text[..],
            UTF8,
            0,
            text.len() as u64,
            &IndexOptions::default(),
        )
        .unwrap();
        let mut f = Follow::after_build(UTF8, &index, text.len() as u64);

        assert_eq!(
            f.poll(&text[..], &mut index, text.len() as u64).unwrap(),
            Poll::Unchanged
        );
        assert_eq!(index.line_count(), 2);

        // §5.5's truncation, which this module deliberately does not handle.
        assert_eq!(
            f.poll(&text[..], &mut index, 4).unwrap(),
            Poll::Shrank {
                was: text.len() as u64,
                now: 4
            }
        );
        assert_eq!(index.line_count(), 2, "a shrink must not touch the index");
    }

    /// **§11.3: a tick is bounded, and the caller is told to come back.**
    #[test]
    fn a_large_append_is_split_across_ticks_and_says_so() {
        let mut text = String::from("first\n");
        // Comfortably more than one budget's worth.
        text.push_str(&"x".repeat(FOLLOW_BUDGET_BYTES as usize + 1024));
        text.push('\n');
        let bytes = text.as_bytes();

        let mut index = build_index(&bytes[..6], UTF8, 0, 6, &IndexOptions::default()).unwrap();
        let mut f = Follow::after_build(UTF8, &index, 6);

        let first = f.poll(bytes, &mut index, bytes.len() as u64).unwrap();
        assert!(
            matches!(first, Poll::Grew { more: true, .. }),
            "a {}-byte append fitted in one tick: {first:?}",
            bytes.len()
        );
        assert!(
            f.scanned_to() - 6 <= FOLLOW_BUDGET_BYTES,
            "the budget was exceeded"
        );

        let mut guard = 0;
        while let Poll::Grew { more: true, .. } =
            f.poll(bytes, &mut index, bytes.len() as u64).unwrap()
        {
            guard += 1;
            assert!(guard < 100, "never finished");
        }
        assert_eq!(index.line_count(), oracle(bytes).line_count());
    }
}
