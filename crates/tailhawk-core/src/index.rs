//! The line-offset index — `SPEC.md` §5.3, as re-derived clean-room on 2026-08-04.
//!
//! Two pieces, deliberately separate:
//!
//! - [`LineScanner`] finds line starts in a byte stream, carrying a partially-matched terminator
//!   across chunk boundaries. It does no I/O and holds no index.
//! - [`LineIndex`] stores every `stride`-th line start and nothing else. It does no I/O and does no
//!   scanning.
//!
//! Keeping them apart is what lets the parallel indexer (E4) run a scanner per chunk without any
//! shared mutable state, and lets both be tested without a file.
//!
//! **Why sparse rather than one offset per line:** at 10 GB and ~100 B/line, storing every line
//! costs 800 MB. Storing every 64th costs 12.5 MB, and the forward scan that recovers the lines
//! between is 6.3 KB — under two pages. `SPEC.md` §5.3 has the full derivation and the table that
//! rejects the delta-encoded alternatives.

use crate::encoding::Charset;

/// Lines between stored anchors.
///
/// `SPEC.md` §5.3 chooses this against **cold-read latency**, not memory: every sparse stride is
/// already negligible against §11.2's budget, so the figure that matters is how much must be read
/// on a random seek. 64 lines is ~6.3 KB at the measured line lengths.
pub const ANCHOR_STRIDE: u64 = 64;

/// Anchors per allocation block — 4,096 × `u64` = 32 KB.
///
/// A followed file grows without bound. Appending to one flat vector of tens of millions of entries
/// reallocates and copies the lot at exactly the moment the UI is trying to hold 60 Hz, so anchors
/// live in fixed-size blocks that are never resized after allocation.
const ANCHORS_PER_BLOCK: usize = 4096;

/// A stored line start: the line's number and its absolute byte offset.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub line: u64,
    pub offset: u64,
}

/// A run of lines whose anchors are regularly spaced from the run's own first line.
///
/// A serially-built index is one segment. The parallel indexer (E4) adds one per chunk, because a
/// worker cannot know the global line number its chunk starts at — that is only settled once every
/// earlier chunk has finished counting — so it anchors from its own first line and the merge
/// records the base. `SPEC.md` §5.3's "prefix sum over the per-chunk line counts converts local
/// anchor numbering to global" is this field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct Segment {
    base_line: u64,
    first_anchor: u64,
}

/// Sparse line starts, in append-only fixed-size blocks.
#[derive(Debug)]
pub struct LineIndex {
    stride: u64,
    blocks: Vec<Vec<u64>>,
    /// Ascending by `base_line`, and never empty once any line exists.
    segments: Vec<Segment>,
    anchors: u64,
    lines: u64,
}

impl Default for LineIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl LineIndex {
    pub fn new() -> Self {
        Self::with_stride(ANCHOR_STRIDE)
    }

    /// A stride of 1 indexes every line, which is what the tests use to check the sparse path
    /// against an exhaustive one.
    ///
    /// # Panics
    /// If `stride` is zero.
    pub fn with_stride(stride: u64) -> Self {
        assert!(stride > 0, "a stride of zero would index no lines at all");
        Self {
            stride,
            blocks: Vec::new(),
            segments: Vec::new(),
            anchors: 0,
            lines: 0,
        }
    }

    /// Records the start of the next line. Call once per line, in order.
    ///
    /// The caller owns line numbering: the *n*-th call is line *n*. Line 0 starts after the byte
    /// order mark, not at zero, which is why this takes an offset rather than deriving one.
    pub fn push_line(&mut self, offset: u64) {
        let base = match self.segments.last() {
            Some(s) => s.base_line,
            None => {
                self.segments.push(Segment {
                    base_line: self.lines,
                    first_anchor: self.anchors,
                });
                self.lines
            }
        };
        if (self.lines - base).is_multiple_of(self.stride) {
            self.push_anchor(offset);
        }
        self.lines += 1;
    }

    /// Appends a whole chunk's worth of lines, anchored from the chunk's own first line.
    ///
    /// This is the merge half of the parallel indexer: `anchors` are the offsets of the chunk's
    /// local lines `0, stride, 2·stride, …`, and the chunk's base line number is however many lines
    /// have already been appended. Chunks must be merged **in file order**.
    ///
    /// # Panics
    /// If `anchors` does not hold exactly one entry per stride of `lines` — that mismatch means a
    /// worker and the merge disagree about the stride, which would silently misplace every lookup
    /// in the chunk.
    pub fn append_chunk(&mut self, lines: u64, anchors: &[u64]) {
        assert_eq!(
            anchors.len() as u64,
            lines.div_ceil(self.stride),
            "a chunk of {lines} lines at stride {} needs {} anchors, not {}",
            self.stride,
            lines.div_ceil(self.stride),
            anchors.len()
        );
        if lines == 0 {
            return;
        }
        self.segments.push(Segment {
            base_line: self.lines,
            first_anchor: self.anchors,
        });
        for &offset in anchors {
            self.push_anchor(offset);
        }
        self.lines += lines;
    }

    fn push_anchor(&mut self, offset: u64) {
        if self
            .blocks
            .last()
            .is_none_or(|b| b.len() == ANCHORS_PER_BLOCK)
        {
            self.blocks.push(Vec::with_capacity(ANCHORS_PER_BLOCK));
        }
        // `last_mut` cannot be None: the branch above guarantees a block with room.
        self.blocks
            .last_mut()
            .expect("a block was just ensured")
            .push(offset);
        self.anchors += 1;
    }

    fn pop_anchor(&mut self) {
        if let Some(block) = self.blocks.last_mut() {
            block.pop();
        }
        if self.blocks.last().is_some_and(|b| b.is_empty()) {
            self.blocks.pop();
        }
        self.anchors = self.anchors.saturating_sub(1);
    }

    fn anchor(&self, nth: u64) -> Option<u64> {
        let nth = usize::try_from(nth).ok()?;
        self.blocks
            .get(nth / ANCHORS_PER_BLOCK)?
            .get(nth % ANCHORS_PER_BLOCK)
            .copied()
    }

    /// Removes the most recently pushed line.
    ///
    /// Needed for exactly one case: a file whose last byte is a terminator produces a line start at
    /// EOF, and that is not a line. The decoder makes the same distinction — see
    /// `LineDecoder::finish` — and the two must agree or the grid and the index disagree about how
    /// many rows exist.
    pub fn pop_line(&mut self) {
        if self.lines == 0 {
            return;
        }
        self.lines -= 1;
        let Some(&Segment { base_line, .. }) = self.segments.last() else {
            return;
        };
        if (self.lines - base_line).is_multiple_of(self.stride) {
            self.pop_anchor();
        }
        if self.lines == base_line {
            self.segments.pop();
        }
    }

    /// Complete lines known so far. A partial index reports a lower bound, which `SPEC.md` §11.3
    /// requires the scrollbar to treat as provisional.
    pub fn line_count(&self) -> u64 {
        self.lines
    }

    pub fn is_empty(&self) -> bool {
        self.lines == 0
    }

    /// The nearest stored line at or before `line` — the point a forward scan starts from.
    ///
    /// Returns `None` only when `line` is beyond what has been indexed, so a caller can tell
    /// "not indexed yet" from "line 0".
    pub fn anchor_at_or_before(&self, line: u64) -> Option<Anchor> {
        if line >= self.lines {
            return None;
        }
        // Every segment's `base_line` is <= `self.lines`, and segment 0's is the first line there
        // is, so a line inside the index always lands in a segment.
        let nth_segment = self.segments.partition_point(|s| s.base_line <= line) - 1;
        let segment = self.segments[nth_segment];
        let local = line - segment.base_line;
        let step = local / self.stride;
        Some(Anchor {
            line: segment.base_line + step * self.stride,
            offset: self.anchor(segment.first_anchor + step)?,
        })
    }

    /// Bytes held by the anchor storage — for asserting the §11.2 budget rather than trusting it.
    pub fn memory_bytes(&self) -> usize {
        self.blocks.capacity() * std::mem::size_of::<Vec<u64>>()
            + self
                .blocks
                .iter()
                .map(|b| b.capacity() * std::mem::size_of::<u64>())
                .sum::<usize>()
            + self.segments.capacity() * std::mem::size_of::<Segment>()
    }

    /// How many separately-anchored runs the index is in — one for a serial build, one per chunk
    /// for a parallel one. Exposed so a test can assert the directory stays small rather than
    /// growing with the file.
    pub fn segment_count(&self) -> usize {
        self.segments.len()
    }

    pub fn stride(&self) -> u64 {
        self.stride
    }
}

/// Finds line starts in a byte stream, carrying a partially-matched terminator across chunks.
///
/// One per chunk in the parallel indexer, or one long-lived one when following.
pub struct LineScanner {
    terminator: &'static [u8],
    code_unit: u64,
    /// Absolute offset of the next byte to be consumed. Absolute, because code-unit alignment is
    /// relative to the start of the *file* — a scanner that counted from its own chunk would call
    /// every second byte pair a terminator in a UTF-16 file split at an odd offset.
    offset: u64,
    /// Bytes of the terminator matched so far, carried from an earlier chunk if need be.
    matched: usize,
}

impl LineScanner {
    /// `start_offset` is the absolute file offset of the first byte that will be fed.
    pub fn new(charset: Charset, start_offset: u64) -> Self {
        Self {
            terminator: charset.line_terminator(),
            code_unit: charset.code_unit() as u64,
            offset: start_offset,
            matched: 0,
        }
    }

    /// Feeds one read's worth of bytes, calling `on_line_start` with the absolute offset of the
    /// first byte **after** each terminator.
    ///
    /// The first line of a file is not preceded by a terminator, so the caller pushes it.
    pub fn push(&mut self, bytes: &[u8], mut on_line_start: impl FnMut(u64)) {
        for &b in bytes {
            let at = self.offset;
            self.offset += 1;

            if self.matched > 0 {
                if b == self.terminator[self.matched] {
                    self.matched += 1;
                    if self.matched == self.terminator.len() {
                        self.matched = 0;
                        on_line_start(self.offset);
                    }
                    continue;
                }
                // A failed continuation. The byte is still eligible to open a fresh match, which
                // matters for `00 00 00 0A` in UTF-32BE where a run of NULs precedes the real one.
                self.matched = 0;
            }

            if at.is_multiple_of(self.code_unit) && b == self.terminator[0] {
                if self.terminator.len() == 1 {
                    on_line_start(self.offset);
                } else {
                    self.matched = 1;
                }
            }
        }
    }

    /// Absolute offset one past the last byte consumed.
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    fn utf16be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    fn utf32le(s: &str) -> Vec<u8> {
        s.chars().flat_map(|c| (c as u32).to_le_bytes()).collect()
    }

    fn utf32be(s: &str) -> Vec<u8> {
        s.chars().flat_map(|c| (c as u32).to_be_bytes()).collect()
    }

    /// Scans `bytes` in fixed-size chunks, returning every line start including the first.
    fn scan(charset: Charset, bytes: &[u8], chunk: usize) -> Vec<u64> {
        let mut starts = vec![0u64];
        let mut s = LineScanner::new(charset, 0);
        for part in bytes.chunks(chunk.max(1)) {
            s.push(part, |o| starts.push(o));
        }
        starts
    }

    #[test]
    fn a_stride_of_one_indexes_every_line() {
        let mut idx = LineIndex::with_stride(1);
        for (n, offset) in [0u64, 10, 25, 40].iter().enumerate() {
            idx.push_line(*offset);
            assert_eq!(idx.line_count(), n as u64 + 1);
        }
        for (line, offset) in [(0u64, 0u64), (1, 10), (2, 25), (3, 40)] {
            assert_eq!(
                idx.anchor_at_or_before(line),
                Some(Anchor { line, offset }),
                "line {line}"
            );
        }
    }

    #[test]
    fn a_sparse_index_returns_the_anchor_at_or_before() {
        let mut idx = LineIndex::with_stride(4);
        for n in 0..10u64 {
            idx.push_line(n * 100);
        }
        // Anchors are lines 0, 4 and 8 only.
        for (query, expected) in [(0u64, 0u64), (1, 0), (3, 0), (4, 4), (7, 4), (8, 8), (9, 8)] {
            let a = idx.anchor_at_or_before(query).expect("within the index");
            assert_eq!(a.line, expected, "querying line {query}");
            assert_eq!(a.offset, expected * 100, "querying line {query}");
        }
    }

    #[test]
    fn a_line_beyond_the_index_is_none_rather_than_a_clamp() {
        let mut idx = LineIndex::with_stride(4);
        assert_eq!(
            idx.anchor_at_or_before(0),
            None,
            "an empty index knows nothing"
        );
        idx.push_line(0);
        assert!(idx.anchor_at_or_before(0).is_some());
        assert_eq!(
            idx.anchor_at_or_before(1),
            None,
            "a partial index must not answer for a line it has not reached"
        );
    }

    #[test]
    fn anchors_survive_crossing_a_block_boundary() {
        let mut idx = LineIndex::with_stride(1);
        let n = ANCHORS_PER_BLOCK as u64 * 2 + 7;
        for i in 0..n {
            idx.push_line(i * 8);
        }
        assert_eq!(idx.line_count(), n);
        for line in [
            0,
            1,
            ANCHORS_PER_BLOCK as u64 - 1,
            ANCHORS_PER_BLOCK as u64,
            n - 1,
        ] {
            assert_eq!(
                idx.anchor_at_or_before(line).map(|a| a.offset),
                Some(line * 8),
                "line {line} spans a block boundary"
            );
        }
    }

    #[test]
    fn pop_line_removes_the_anchor_it_created() {
        let mut idx = LineIndex::with_stride(4);
        for n in 0..9u64 {
            idx.push_line(n * 10);
        }
        assert_eq!(idx.line_count(), 9);
        idx.pop_line();
        assert_eq!(idx.line_count(), 8);
        assert_eq!(
            idx.anchor_at_or_before(8),
            None,
            "line 8 was an anchor and was popped"
        );
        assert_eq!(idx.anchor_at_or_before(7).map(|a| a.line), Some(4));
    }

    #[test]
    fn popping_everything_empties_the_index() {
        let mut idx = LineIndex::with_stride(2);
        for n in 0..5u64 {
            idx.push_line(n);
        }
        for _ in 0..5 {
            idx.pop_line();
        }
        assert!(idx.is_empty());
        idx.pop_line();
        assert!(
            idx.is_empty(),
            "popping an empty index is a no-op, not a panic"
        );
    }

    #[test]
    fn single_byte_encodings_split_on_every_0a() {
        let starts = scan(Charset::UTF_8, b"one\ntwo\nthree", 64);
        assert_eq!(starts, [0, 4, 8]);
    }

    #[test]
    fn a_crlf_line_start_is_after_the_lf_not_the_cr() {
        let starts = scan(Charset::UTF_8, b"one\r\ntwo\r\n", 64);
        assert_eq!(
            starts,
            [0, 5, 10],
            "CR is part of the terminator, so the next line starts after LF"
        );
    }

    /// The silent-corruption class `PLAN.md` marks E4 **High** risk. A `0x0A` at an odd offset in
    /// UTF-16LE is the low byte of some other character, not a terminator.
    #[test]
    fn a_misaligned_0a_is_not_a_terminator_in_utf16() {
        // U+4A00 encodes LE as `00 4A`; U+000A encodes as `0A 00`. A scanner ignoring alignment
        // finds the `0A` of U+4A00 and splits the file in the wrong place.
        let text = "\u{4A00}\u{4A00}\n";
        let bytes = utf16le(text);
        assert!(
            bytes.contains(&0x0A),
            "the fixture must contain a decoy 0x0A"
        );

        let starts = scan(Charset::UTF_16LE, &bytes, 64);
        assert_eq!(
            starts,
            [0, bytes.len() as u64],
            "only the aligned terminator counts; got {starts:?} for {bytes:02x?}"
        );
    }

    #[test]
    fn every_encoding_finds_its_own_terminator() {
        let cases: Vec<(Charset, Vec<u8>, u64)> = vec![
            (Charset::UTF_8, b"ab\ncd".to_vec(), 3),
            (Charset::UTF_16LE, utf16le("ab\ncd"), 6),
            (Charset::UTF_16BE, utf16be("ab\ncd"), 6),
            (Charset::Utf32Le, utf32le("ab\ncd"), 12),
            (Charset::Utf32Be, utf32be("ab\ncd"), 12),
        ];
        for (charset, bytes, second_line) in cases {
            let starts = scan(charset, &bytes, 64);
            assert_eq!(
                starts,
                [0, second_line],
                "{} over {bytes:02x?}",
                charset.name()
            );
        }
    }

    /// UTF-32BE's terminator is `00 00 00 0A`, so a run of NULs is a long partial match that must
    /// be able to restart without swallowing the real terminator behind it.
    #[test]
    fn a_failed_partial_match_can_still_open_a_new_one() {
        let bytes = utf32be("\u{0}\n");
        let starts = scan(Charset::Utf32Be, &bytes, 64);
        assert_eq!(
            starts,
            [0, 8],
            "U+0000 then U+000A: {bytes:02x?} — the NUL must not consume the terminator"
        );
    }

    /// The property that matters for E4: a scan must not depend on where the reads happened to
    /// land, or a parallel indexer and a serial one disagree.
    #[test]
    fn line_starts_are_identical_however_the_bytes_are_chunked() {
        let cases: Vec<(Charset, Vec<u8>)> = vec![
            (Charset::UTF_8, b"alpha\nbeta\r\n\ngamma\n".to_vec()),
            (Charset::UTF_16LE, utf16le("alpha\nbeta\r\n\n\u{4A00}\n")),
            (Charset::UTF_16BE, utf16be("alpha\nbeta\r\n\n\u{4A00}\n")),
            (Charset::Utf32Le, utf32le("alpha\n\u{0}\nbeta\n")),
            (Charset::Utf32Be, utf32be("alpha\n\u{0}\nbeta\n")),
        ];
        for (charset, bytes) in cases {
            let reference = scan(charset, &bytes, bytes.len().max(1));
            for chunk in 1..=9 {
                assert_eq!(
                    scan(charset, &bytes, chunk),
                    reference,
                    "{} chunked {chunk} bytes at a time, over {bytes:02x?}",
                    charset.name()
                );
            }
        }
    }

    /// The index and the decoder must agree on how many lines a file has. They compute it by
    /// completely different routes — one scans bytes, the other decodes characters — so this is a
    /// real cross-check rather than a restatement.
    #[test]
    fn the_index_line_count_agrees_with_the_decoder() {
        use crate::lines::LineDecoder;

        for text in [
            "one\ntwo\nthree",   // no trailing terminator
            "one\ntwo\nthree\n", // trailing terminator
            "",                  // empty
            "\n",                // a single empty line
            "\n\n\n",            // empty lines are lines
            "one\r\ntwo\r\n",    // CRLF
        ] {
            let bytes = text.as_bytes();

            let mut decoded = 0u64;
            let mut d = LineDecoder::new(Charset::UTF_8);
            d.push(bytes, |_| decoded += 1);
            d.finish(|_| decoded += 1);

            let mut idx = LineIndex::with_stride(1);
            if !bytes.is_empty() {
                idx.push_line(0);
                let mut s = LineScanner::new(Charset::UTF_8, 0);
                s.push(bytes, |o| idx.push_line(o));
                // A terminator at EOF opens a line that has no content and does not exist.
                if bytes.last() == Some(&b'\n') {
                    idx.pop_line();
                }
            }

            assert_eq!(
                idx.line_count(),
                decoded,
                "index and decoder disagree on {text:?}"
            );
        }
    }

    /// §11.2 budgets the index by *bytes per line*. This asserts the structure actually delivers
    /// the 0.125 the table claims, rather than the table being aspirational.
    #[test]
    fn the_index_costs_what_section_11_2_says_it_does() {
        let lines = 1_000_000u64;
        let mut idx = LineIndex::new();
        for n in 0..lines {
            idx.push_line(n * 100);
        }
        let per_line = idx.memory_bytes() as f64 / lines as f64;
        assert!(
            per_line < 0.14,
            "index costs {per_line:.4} bytes/line; §11.2 claims 0.125 at stride {}",
            idx.stride()
        );
    }
}
