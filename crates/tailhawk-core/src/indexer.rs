//! Parallel index construction — E4, to `SPEC.md` §5.3 "Construction".
//!
//! The file is divided into code-unit-aligned chunks and one [`LineScanner`] runs per chunk. No
//! worker reads another's result: each counts its own lines and anchors them from its own first
//! line, and the merge turns local numbering into global with a prefix sum over the counts. That is
//! why [`LineIndex`] carries a segment directory — a worker cannot know its chunk's global line
//! number until every earlier chunk has finished counting.
//!
//! **Why an aligned boundary is enough.** `Charset::line_terminator()` is always exactly
//! `Charset::code_unit()` bytes long, so a boundary at a multiple of the code unit can never fall
//! *inside* a terminator: every terminator lies wholly within one chunk. A worker therefore starts
//! with no carried match, and a chunk scanned alone gives the same line starts as the same bytes
//! scanned as part of a whole-file pass. `a_terminator_is_never_split_by_an_aligned_boundary` holds
//! that property down.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::encoding::Charset;
use crate::index::{Extent, LineIndex, LineScanner, ANCHOR_STRIDE};
use crate::{Error, Result};

/// Positional reads, which is all the indexer needs of a source.
///
/// `Sync` because every worker reads through the same reference. `LogFile` qualifies: each read
/// carries its own `OVERLAPPED` with an explicit offset, so there is no shared file pointer to
/// contend for (`SPEC.md` §5.2).
pub trait ChunkReader: Sync {
    /// Reads at an absolute offset, returning the byte count. Short at end of file, zero past it.
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize>;
}

#[cfg(windows)]
impl ChunkReader for crate::file::LogFile {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        Ok(crate::file::LogFile::read_at(self, offset, buf)?)
    }
}

/// An in-memory source. The tests index without a file at all, and `SPEC.md` §4.2's stdin spill
/// will want the same shape.
impl ChunkReader for [u8] {
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let Ok(offset) = usize::try_from(offset) else {
            return Ok(0);
        };
        if offset >= self.len() {
            return Ok(0);
        }
        let n = buf.len().min(self.len() - offset);
        buf[..n].copy_from_slice(&self[offset..offset + n]);
        Ok(n)
    }
}

/// Bytes of file per worker task. Work assignment, not I/O size.
const CHUNK_BYTES: u64 = 8 * 1024 * 1024;

/// Bytes per `read_at` while scanning. Peak indexing memory is this times the thread count.
const READ_BYTES: usize = 256 * 1024;

/// Bytes per `read_at` while resolving one line from its anchor. §5.3 puts the expected forward
/// scan at 6.3 KB, so this is one comfortable read rather than a tuned figure.
const RESOLVE_READ_BYTES: usize = 64 * 1024;

#[derive(Copy, Clone, Debug)]
pub struct IndexOptions {
    pub stride: u64,
    pub chunk_bytes: u64,
    pub read_bytes: usize,
    pub threads: usize,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            stride: ANCHOR_STRIDE,
            chunk_bytes: CHUNK_BYTES,
            read_bytes: READ_BYTES,
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        }
    }
}

/// One chunk's contribution, in the worker's own local line numbering.
#[derive(Debug, Default)]
struct ChunkScan {
    lines: u64,
    /// Offsets of local lines `0, stride, 2·stride, …`.
    anchors: Vec<u64>,
    /// The last line start found, so the merge can tell a terminator at end of file from a line.
    last_start: Option<u64>,
    /// This chunk's horizontal extent (§3.3), joined to its neighbours in file order.
    extent: Extent,
    /// One past the last byte actually read, or `None` if the chunk read nothing at all.
    ///
    /// This is what the trailing-terminator test compares against, and it is **not** the same as the
    /// chunk's nominal end: `end` was sampled before the scan, and a writer that copy-truncates
    /// (§5.5) can leave the file shorter than that. Testing against the nominal end would miss the
    /// terminator and invent a line at a byte that no longer exists.
    data_end: Option<u64>,
}

/// Indexes `[start, end)` in parallel.
///
/// `start` is the first byte after any byte-order mark; `end` is the file length at the moment
/// indexing began. Following past `end` appends through [`LineIndex::push_line`] — this builds the
/// index that exists when the file is opened, not the one that grows afterwards.
pub fn build_index<R: ChunkReader + ?Sized>(
    reader: &R,
    charset: Charset,
    start: u64,
    end: u64,
    opts: &IndexOptions,
) -> Result<LineIndex> {
    let code_unit = charset.code_unit() as u64;
    if opts.stride == 0 || opts.chunk_bytes == 0 || opts.read_bytes == 0 {
        return Err(Error(
            "an index needs a non-zero stride, chunk and read size".into(),
        ));
    }
    if !start.is_multiple_of(code_unit) || !opts.chunk_bytes.is_multiple_of(code_unit) {
        // Splitting a UTF-16 file at an odd offset swaps every subsequent byte pair, so the scan
        // hunts `0A 00` in a stream where it is now written `00 0A` (§5.3).
        return Err(Error(format!(
            "a chunk boundary must be a multiple of {code_unit} bytes in {}, but start={start} \
             and chunk={} are not",
            charset.name(),
            opts.chunk_bytes
        )));
    }

    let mut index = LineIndex::with_stride(opts.stride);
    if end <= start {
        return Ok(index);
    }

    let chunks: Vec<(u64, u64)> = (start..end)
        .step_by(opts.chunk_bytes as usize)
        // Saturating because `chunk_bytes` is caller-supplied: a chunk size near `u64::MAX` would
        // otherwise overflow to a `to` below `from`, and the chunk's lines would vanish silently
        // in release, where the add does not panic.
        .map(|from| (from, from.saturating_add(opts.chunk_bytes).min(end)))
        .collect();

    let next = AtomicUsize::new(0);
    let threads = opts.threads.clamp(1, chunks.len());
    let mut scans: Vec<Option<ChunkScan>> = (0..chunks.len()).map(|_| None).collect();

    std::thread::scope(|scope| -> Result<()> {
        let mut workers = Vec::with_capacity(threads);
        for _ in 0..threads {
            let (next, chunks, opts) = (&next, &chunks, &opts);
            workers.push(scope.spawn(move || -> Result<Vec<(usize, ChunkScan)>> {
                let mut buf = vec![0u8; opts.read_bytes];
                let mut done = Vec::new();
                loop {
                    let nth = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(from, to)) = chunks.get(nth) else {
                        return Ok(done);
                    };
                    // Only the first chunk owns the file's opening line: every other line starts
                    // after a terminator, and that terminator belongs to whichever chunk holds it.
                    let scan =
                        scan_chunk(reader, charset, from, to, opts.stride, nth == 0, &mut buf)?;
                    done.push((nth, scan));
                }
            }));
        }
        for worker in workers {
            let done = worker
                .join()
                .map_err(|_| Error("an indexing worker panicked".into()))??;
            for (nth, scan) in done {
                scans[nth] = Some(scan);
            }
        }
        Ok(())
    })?;

    let mut final_start = None;
    let mut data_end = None;
    // §3.3's horizontal extent. Folded in file order rather than max-ed, because a line cut in two
    // by a chunk boundary is one line — see `Extent::merge`.
    let mut extent = Extent::EMPTY;
    for (nth, scan) in scans.iter_mut().enumerate() {
        let scan = scan
            .take()
            .ok_or_else(|| Error(format!("chunk {nth} was never scanned")))?;
        final_start = scan.last_start.or(final_start);
        data_end = scan.data_end.or(data_end);
        extent = extent.merge(scan.extent);
        index.append_chunk(scan.lines, &scan.anchors);
    }
    index.set_extent(extent);

    // A terminator as the file's last bytes opens a line start at end of data, and that is not a
    // line. `LineDecoder::finish` draws the same distinction, and the two must agree or the grid
    // and the index disagree about how many rows exist.
    //
    // Compared against the bytes actually read, never against `end`: `end` was sampled before the
    // scan, and a file truncated in between is shorter than it. Testing `end` there would leave the
    // phantom line in place — the exact disagreement the paragraph above exists to prevent.
    if final_start.is_some() && final_start == data_end {
        index.pop_line();
    }

    Ok(index)
}

/// Scans one chunk, anchoring from the chunk's own first line.
fn scan_chunk<R: ChunkReader + ?Sized>(
    reader: &R,
    charset: Charset,
    from: u64,
    to: u64,
    stride: u64,
    owns_first_line: bool,
    buf: &mut [u8],
) -> Result<ChunkScan> {
    let mut scan = ChunkScan::default();
    let mut scanner = LineScanner::new(charset, from);
    let mut at = from;
    while at < to {
        let want = usize::try_from(to - at).unwrap_or(buf.len()).min(buf.len());
        let read = reader.read_at(at, &mut buf[..want])?;
        if read == 0 {
            // The file shrank under us. Rotation and truncation are M4's (§5.5); all this owes is
            // to stop rather than spin — and to not claim bytes it never saw, which is why the
            // opening line is seeded below rather than up front.
            break;
        }
        if owns_first_line && scan.lines == 0 {
            // The file's opening line, seeded only now that a byte is known to exist. A file that
            // was sized and then truncated to nothing has no first line to own.
            scan.lines = 1;
            scan.anchors.push(from);
            scan.last_start = Some(from);
        }
        scanner.push(&buf[..read], |offset| {
            if scan.lines.is_multiple_of(stride) {
                scan.anchors.push(offset);
            }
            scan.lines += 1;
            scan.last_start = Some(offset);
        });
        at += read as u64;
        scan.data_end = Some(at);
    }

    scan.extent = scanner.extent();
    Ok(scan)
}

/// Resolves a line number to its byte offset — R1 of §5.3, and the reason the index exists.
///
/// Takes the anchor at or before `line` and scans forward for the terminators between. `None` means
/// the line is beyond what has been indexed, which a partial index (R5) must be able to say.
pub fn offset_of_line<R: ChunkReader + ?Sized>(
    reader: &R,
    charset: Charset,
    index: &LineIndex,
    line: u64,
) -> Result<Option<u64>> {
    let Some(anchor) = index.anchor_at_or_before(line) else {
        return Ok(None);
    };
    if anchor.line == line {
        return Ok(Some(anchor.offset));
    }

    let mut remaining = line - anchor.line;
    let mut found = None;
    let mut scanner = LineScanner::new(charset, anchor.offset);
    let mut buf = vec![0u8; RESOLVE_READ_BYTES];
    let mut at = anchor.offset;

    while found.is_none() {
        let read = reader.read_at(at, &mut buf)?;
        if read == 0 {
            break;
        }
        scanner.push(&buf[..read], |offset| {
            if found.is_none() {
                remaining -= 1;
                if remaining == 0 {
                    found = Some(offset);
                }
            }
        });
        at += read as u64;
    }

    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lines::LineDecoder;

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    fn utf16be(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    }

    fn utf32le(s: &str) -> Vec<u8> {
        s.chars().flat_map(|c| (c as u32).to_le_bytes()).collect()
    }

    fn options(stride: u64, chunk_bytes: u64, threads: usize) -> IndexOptions {
        IndexOptions {
            stride,
            chunk_bytes,
            read_bytes: 64,
            threads,
        }
    }

    /// Every line start in the file, the slow and obvious way, for the parallel build to be
    /// measured against.
    fn line_starts(charset: Charset, bytes: &[u8]) -> Vec<u64> {
        if bytes.is_empty() {
            return Vec::new();
        }
        let mut starts = vec![0u64];
        LineScanner::new(charset, 0).push(bytes, |offset| starts.push(offset));
        if starts.last() == Some(&(bytes.len() as u64)) {
            starts.pop();
        }
        starts
    }

    /// Resolves every line through the index and compares against the reference.
    fn assert_index_finds_every_line(charset: Charset, bytes: &[u8], index: &LineIndex) {
        let expected = line_starts(charset, bytes);
        assert_eq!(
            index.line_count(),
            expected.len() as u64,
            "line count over {} bytes of {}",
            bytes.len(),
            charset.name()
        );
        for (line, want) in expected.iter().enumerate() {
            let got = offset_of_line(bytes, charset, index, line as u64).expect("in-memory read");
            assert_eq!(got, Some(*want), "line {line} of {}", charset.name());
        }
        assert_eq!(
            offset_of_line(bytes, charset, index, expected.len() as u64).expect("in-memory read"),
            None,
            "one line past the end must be None rather than a clamp"
        );
    }

    /// The longest line in bytes, worked out the slow and obvious way, for the scan to be measured
    /// against.
    fn longest_line(charset: Charset, bytes: &[u8]) -> u64 {
        let starts = line_starts(charset, bytes);
        let terminator = charset.line_terminator();
        let term_len = terminator.len() as u64;
        // The final line ends before a trailing terminator, if there is one. Counting that
        // terminator as content is what the first version of this helper did, and it reported the
        // one-byte file "\n" as having a one-byte line rather than an empty one.
        let last_end = bytes.len() as u64
            - if bytes.ends_with(terminator) {
                term_len
            } else {
                0
            };
        let mut longest = 0u64;
        for (i, start) in starts.iter().enumerate() {
            let end = match starts.get(i + 1) {
                Some(next) => next - term_len,
                None => last_end,
            };
            longest = longest.max(end - start);
        }
        longest
    }

    /// **The extent must agree with an independent measurement at every chunk size and thread
    /// count.** The interesting case is a line longer than a chunk: the scan sees only fragments of
    /// it, and taking the max of those fragments would *understate* the answer — which would make
    /// §3.3's "upper bound" not one, and let content be wider than the scrollbar admits.
    #[test]
    fn the_horizontal_extent_agrees_with_an_independent_scan() {
        let long = "x".repeat(500);
        let corpus = format!("short\na bit longer\n{long}\ntail\n{long}{long}\nlast line here");
        let bytes = corpus.as_bytes();
        let want = longest_line(Charset::UTF_8, bytes);
        assert_eq!(want, 1000, "the fixture should contain a 1,000-byte line");

        for chunk_bytes in [1u64, 2, 7, 64, 300, 1024, 4096] {
            for threads in [1usize, 2, 8] {
                let index = build_index(
                    bytes,
                    Charset::UTF_8,
                    0,
                    bytes.len() as u64,
                    &options(4, chunk_bytes, threads),
                )
                .expect("build");
                assert_eq!(
                    index.extent().max_line_bytes(),
                    want,
                    "chunk {chunk_bytes}, {threads} threads: a line spanning chunks was not stitched"
                );
            }
        }
    }

    /// The bound must never be *below* the true longest line, whatever the input — that is the
    /// direction that matters, because an understated bound clips content out of reach.
    #[test]
    fn the_extent_is_never_an_underestimate() {
        let cases: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"\n".to_vec(),
            b"\n\n\n".to_vec(),
            b"no terminator at all".to_vec(),
            b"a\nbb\nccc".to_vec(),
            b"trailing\n".to_vec(),
            "x".repeat(300).into_bytes(),
            format!("{}\n{}", "y".repeat(200), "z".repeat(199)).into_bytes(),
        ];
        for bytes in &cases {
            let want = longest_line(Charset::UTF_8, bytes);
            for chunk_bytes in [1u64, 3, 16, 512] {
                for threads in [1usize, 4] {
                    let index = build_index(
                        &bytes[..],
                        Charset::UTF_8,
                        0,
                        bytes.len() as u64,
                        &options(2, chunk_bytes, threads),
                    )
                    .expect("build");
                    let got = index.extent().max_line_bytes();
                    assert!(
                        got >= want,
                        "{:?} at chunk {chunk_bytes}/{threads} threads: bound {got} is below the \
                         true longest line {want}",
                        String::from_utf8_lossy(bytes)
                    );
                    assert_eq!(got, want, "and it should be tight, not merely safe");
                }
            }
        }
    }

    /// `all_ascii` is a statement about bytes, and `exact_cells` is where the encoding is resolved.
    /// A UTF-16LE file of pure ASCII has all its bytes below `0x80` — the high halves are `0x00` —
    /// but its byte length is twice its cell count, so it must **not** claim to be exact.
    #[test]
    fn only_a_byte_oriented_all_ascii_run_gives_an_exact_cell_count() {
        let ascii = b"hello there\nshort\n";
        let index = build_index(
            &ascii[..],
            Charset::UTF_8,
            0,
            ascii.len() as u64,
            &options(2, 4, 2),
        )
        .expect("build");
        assert!(index.extent().all_ascii());
        assert_eq!(
            index.extent().exact_cells(Charset::UTF_8),
            Some(11),
            "an all-ASCII UTF-8 run knows its own cell count"
        );

        let wide = "héllo thère\nshort\n".as_bytes();
        let index = build_index(
            wide,
            Charset::UTF_8,
            0,
            wide.len() as u64,
            &options(2, 4, 2),
        )
        .expect("build");
        assert!(!index.extent().all_ascii(), "é is not ASCII");
        assert_eq!(index.extent().exact_cells(Charset::UTF_8), None);
        assert!(
            index.extent().max_line_bytes() >= 11,
            "but the byte bound still covers it"
        );

        let utf16 = utf16le("hello there\nshort\n");
        let index = build_index(
            &utf16[..],
            Charset::UTF_16LE,
            0,
            utf16.len() as u64,
            &options(2, 4, 2),
        )
        .expect("build");
        assert!(
            index.extent().all_ascii(),
            "UTF-16LE ASCII really does have every byte below 0x80"
        );
        assert_eq!(
            index.extent().exact_cells(Charset::UTF_16LE),
            None,
            "but 22 bytes is 11 cells, so the byte length is not the cell count"
        );
        assert_eq!(index.extent().max_line_bytes(), 22);
    }

    /// The invariant the whole parallel scan rests on: a code-unit-aligned boundary cannot fall
    /// inside a terminator, because a terminator is exactly one code unit long. If an encoding ever
    /// arrives where that is false, chunked scanning needs a carry between workers and this test is
    /// what says so.
    #[test]
    fn a_terminator_is_never_split_by_an_aligned_boundary() {
        for charset in [
            Charset::UTF_8,
            Charset::UTF_16LE,
            Charset::UTF_16BE,
            Charset::Utf32Le,
            Charset::Utf32Be,
            Charset::Whatwg(encoding_rs::WINDOWS_1252),
            Charset::Whatwg(encoding_rs::SHIFT_JIS),
            Charset::Whatwg(encoding_rs::BIG5),
            Charset::Whatwg(encoding_rs::EUC_KR),
            Charset::Whatwg(encoding_rs::GBK),
            Charset::Whatwg(encoding_rs::ISO_2022_JP),
        ] {
            assert_eq!(
                charset.line_terminator().len(),
                charset.code_unit(),
                "{} would let an aligned chunk boundary split its terminator",
                charset.name()
            );
        }
    }

    #[test]
    fn a_parallel_build_finds_the_same_lines_as_a_serial_scan() {
        let text: String = (0..500)
            .map(|n| match n % 5 {
                0 => format!("{n} short\n"),
                1 => format!("{n} {}\n", "long ".repeat(40)),
                2 => "\n".to_string(),
                3 => format!("{n} crlf\r\n"),
                _ => format!("{n} \u{4A00}\u{2014} wide\n"),
            })
            .collect();

        let cases: Vec<(Charset, Vec<u8>)> = vec![
            (Charset::UTF_8, text.as_bytes().to_vec()),
            (Charset::UTF_16LE, utf16le(&text)),
            (Charset::UTF_16BE, utf16be(&text)),
            (Charset::Utf32Le, utf32le(&text)),
        ];

        for (charset, bytes) in cases {
            for threads in [1, 2, 3, 8] {
                for chunk in [64u64, 256, 4096, bytes.len() as u64 + 8] {
                    let chunk = chunk.next_multiple_of(charset.code_unit() as u64);
                    let index = build_index(
                        &bytes[..],
                        charset,
                        0,
                        bytes.len() as u64,
                        &options(8, chunk, threads),
                    )
                    .expect("in-memory build");
                    assert_index_finds_every_line(charset, &bytes, &index);
                }
            }
        }
    }

    /// The M2 done-criterion in miniature: the same bytes, indexed on many threads and on one,
    /// resolve every line to the same offset. Run at stride 1 so the anchors themselves are
    /// compared, not just what a forward scan recovers from them.
    #[test]
    fn many_threads_and_one_thread_agree_anchor_for_anchor() {
        let text: String = (0..2000)
            .map(|n| format!("line {n} of the fixture\n"))
            .collect();
        let bytes = utf16le(&text);

        let serial = build_index(
            &bytes[..],
            Charset::UTF_16LE,
            0,
            bytes.len() as u64,
            &options(1, bytes.len() as u64 + 2, 1),
        )
        .expect("serial build");

        let parallel = build_index(
            &bytes[..],
            Charset::UTF_16LE,
            0,
            bytes.len() as u64,
            &options(1, 512, 8),
        )
        .expect("parallel build");

        assert_eq!(serial.line_count(), parallel.line_count());
        for line in 0..serial.line_count() {
            assert_eq!(
                serial.anchor_at_or_before(line),
                parallel.anchor_at_or_before(line),
                "line {line}"
            );
        }
        assert_eq!(
            serial.segment_count(),
            1,
            "a single-chunk build is one segment, so it is bit-identical to a serial one"
        );
    }

    /// A `0x0A` at an odd offset in UTF-16LE is the low byte of another character. The chunked path
    /// has to keep getting this right even when a chunk boundary lands in the middle of the decoy.
    #[test]
    fn a_misaligned_0a_survives_every_chunk_boundary() {
        let bytes = utf16le("\u{4A00}\u{4A00}\n\u{4A00}\n\u{4A00}\u{4A00}\u{4A00}\n");
        assert!(bytes.contains(&0x0A), "the fixture must contain a decoy");

        for chunk in (2..=bytes.len() as u64 + 2).step_by(2) {
            for threads in [1, 4] {
                let index = build_index(
                    &bytes[..],
                    Charset::UTF_16LE,
                    0,
                    bytes.len() as u64,
                    &options(2, chunk, threads),
                )
                .expect("in-memory build");
                assert_index_finds_every_line(Charset::UTF_16LE, &bytes, &index);
            }
        }
    }

    /// Empty lines are the worst case for a per-chunk buffer of line starts, which is the design
    /// this one is built to avoid. It is also the case where a chunk is nothing but terminators.
    #[test]
    fn a_file_of_nothing_but_terminators_indexes_correctly() {
        let bytes = vec![b'\n'; 1000];
        for chunk in [8u64, 64, 512] {
            let index = build_index(
                &bytes[..],
                Charset::UTF_8,
                0,
                bytes.len() as u64,
                &options(16, chunk, 4),
            )
            .expect("in-memory build");
            assert_eq!(index.line_count(), 1000, "each terminator ends a line");
            assert_index_finds_every_line(Charset::UTF_8, &bytes, &index);
        }
    }

    #[test]
    fn a_chunk_with_no_terminator_contributes_no_segment() {
        // One line long enough to span several chunks, so every chunk but the first is silent.
        let bytes = format!("{}\n", "x".repeat(4000)).into_bytes();
        let index = build_index(
            &bytes[..],
            Charset::UTF_8,
            0,
            bytes.len() as u64,
            &options(16, 64, 4),
        )
        .expect("in-memory build");
        assert_eq!(index.line_count(), 1);
        assert_eq!(
            index.segment_count(),
            1,
            "62 silent chunks must not each leave a directory entry"
        );
    }

    #[test]
    fn a_trailing_terminator_does_not_open_a_line() {
        for (text, lines) in [
            ("one\ntwo\nthree", 3u64),
            ("one\ntwo\nthree\n", 3),
            ("\n", 1),
            ("\n\n\n", 3),
            ("one\r\ntwo\r\n", 2),
            ("x", 1),
        ] {
            let bytes = text.as_bytes();
            for chunk in [1u64, 2, 4, 64] {
                let index = build_index(
                    bytes,
                    Charset::UTF_8,
                    0,
                    bytes.len() as u64,
                    &options(4, chunk, 3),
                )
                .expect("in-memory build");
                assert_eq!(index.line_count(), lines, "{text:?} chunked {chunk}");
                assert_index_finds_every_line(Charset::UTF_8, bytes, &index);
            }
        }
    }

    /// The same cross-check E3 has for the serial path: the index scans bytes, the decoder decodes
    /// characters, and the parallel build must still agree with it.
    #[test]
    fn a_parallel_index_agrees_with_the_decoder_on_the_line_count() {
        for text in ["", "\n", "a\nb\nc", "a\nb\nc\n", "\r\n\r\n", "one\n\ntwo\n"] {
            let bytes = text.as_bytes();

            let mut decoded = 0u64;
            let mut decoder = LineDecoder::new(Charset::UTF_8);
            decoder.push(bytes, |_| decoded += 1);
            decoder.finish(|_| decoded += 1);

            let index = build_index(
                bytes,
                Charset::UTF_8,
                0,
                bytes.len() as u64,
                &options(2, 2, 4),
            )
            .expect("in-memory build");

            assert_eq!(
                index.line_count(),
                decoded,
                "index and decoder disagree on {text:?}"
            );
        }
    }

    /// The byte-order mark occupies its offsets, so line 0 starts after it and every chunk boundary
    /// stays aligned — a UTF-16 BOM is two bytes, a UTF-32 one four.
    #[test]
    fn indexing_starts_after_the_byte_order_mark() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&utf16le("alpha\nbeta\n"));

        let index = build_index(
            &bytes[..],
            Charset::UTF_16LE,
            2,
            bytes.len() as u64,
            &options(1, 8, 2),
        )
        .expect("in-memory build");

        assert_eq!(index.line_count(), 2);
        assert_eq!(
            index.anchor_at_or_before(0).map(|a| a.offset),
            Some(2),
            "line 0 starts after the BOM, not at zero"
        );
        assert_eq!(
            index.anchor_at_or_before(1).map(|a| a.offset),
            Some(2 + utf16le("alpha\n").len() as u64)
        );
    }

    #[test]
    fn a_misaligned_start_is_refused_rather_than_silently_wrong() {
        let bytes = utf16le("alpha\nbeta\n");
        let err = build_index(
            &bytes[..],
            Charset::UTF_16LE,
            1,
            bytes.len() as u64,
            &IndexOptions::default(),
        )
        .expect_err("an odd start in UTF-16 swaps every byte pair");
        assert!(err.0.contains("multiple of 2"), "{}", err.0);

        let err = build_index(
            &bytes[..],
            Charset::UTF_16LE,
            0,
            bytes.len() as u64,
            &options(4, 7, 2),
        )
        .expect_err("an odd chunk size in UTF-16 misaligns every boundary but the first");
        assert!(err.0.contains("multiple of 2"), "{}", err.0);
    }

    /// Everything above indexes a byte slice. This one drives the reader that actually ships —
    /// `LogFile`, on a real file, on however many threads the machine has — because concurrent
    /// positional reads through one handle are a property of `LogFile`, not of the indexer, and no
    /// in-memory reader can test it.
    ///
    /// It is the M2 done-criterion in the small: a UTF-16LE file indexed on many threads agrees
    /// with a single-threaded build, and with the decoder, line for line.
    #[cfg(windows)]
    #[test]
    fn a_real_file_indexes_the_same_on_many_threads_as_on_one() {
        use crate::file::tests::Scratch;
        use crate::file::{FileSource, LogFile};

        let text: String = (0..20_000)
            .map(|n| match n % 3 {
                0 => format!("{n} \u{4A00}\u{4A00} a decoy 0x0A rides in these\n"),
                1 => String::from("\n"),
                _ => format!("{n} {}\n", "padding ".repeat(n % 17)),
            })
            .collect();

        let scratch = Scratch::new("parallel-index");
        let path = scratch.join("indexed.log");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&utf16le(&text));
        std::fs::write(&path, &bytes).expect("writing the fixture");

        let mut decoded = 0u64;
        FileSource::open(&path)
            .expect("opening the fixture")
            .read_to_end(|_| decoded += 1)
            .expect("decoding the fixture");

        let file = LogFile::open(&path).expect("opening the fixture");
        let end = file.len().expect("the length");

        let one = build_index(&file, Charset::UTF_16LE, 2, end, &options(64, 4096, 1))
            .expect("serial build");
        let many = build_index(
            &file,
            Charset::UTF_16LE,
            2,
            end,
            &IndexOptions {
                chunk_bytes: 4096,
                ..IndexOptions::default()
            },
        )
        .expect("parallel build");

        assert_eq!(
            one.line_count(),
            decoded,
            "the index disagrees with the decoder"
        );
        assert_eq!(many.line_count(), one.line_count());
        assert!(
            many.segment_count() > 8,
            "the fixture must actually be split into chunks, or this proves nothing"
        );

        for line in (0..one.line_count()).step_by(97) {
            let a = offset_of_line(&file, Charset::UTF_16LE, &one, line).expect("read");
            let b = offset_of_line(&file, Charset::UTF_16LE, &many, line).expect("read");
            assert_eq!(a, b, "line {line} resolves differently in parallel");
            assert!(
                a.is_some_and(|o| o >= 2 && o.is_multiple_of(2)),
                "line {line} landed at {a:?}, which is not an aligned offset after the BOM"
            );
        }
    }

    /// `end` is sampled before the scan starts, so a writer that copy-truncates (§5.5, one of the
    /// three rotation modes M4 must survive) leaves the file shorter than the range being indexed.
    ///
    /// Two things must hold, and neither did in the first version of this file: a range that reads
    /// nothing produces **no** lines rather than one phantom line at `start`, and the
    /// trailing-terminator rule must be tested against the bytes actually read rather than against
    /// `end` — otherwise the terminator goes unnoticed and the index claims a line at a byte that
    /// no longer exists.
    #[test]
    fn a_file_that_shrank_after_it_was_sized_reports_only_what_is_there() {
        // Sized at 40 bytes, then truncated to nothing.
        let index = build_index(&b""[..], Charset::UTF_8, 0, 40, &options(4, 8, 3))
            .expect("in-memory build");
        assert!(
            index.is_empty(),
            "a range that read nothing has no lines, not one at the start offset"
        );
        assert_eq!(index.anchor_at_or_before(0), None);

        // Sized at 40 bytes, then truncated to 4 — which end with a terminator.
        let bytes = b"a\nb\n";
        for chunk in [2u64, 4, 8, 64] {
            let index = build_index(&bytes[..], Charset::UTF_8, 0, 40, &options(4, chunk, 3))
                .expect("in-memory build");
            assert_eq!(
                index.line_count(),
                2,
                "chunked {chunk}: the terminator at the new end of data must not open a third line"
            );
            assert_index_finds_every_line(Charset::UTF_8, bytes, &index);
        }

        // And truncated to 3, which do *not* end with a terminator.
        let index = build_index(&b"a\nb"[..], Charset::UTF_8, 0, 40, &options(4, 2, 3))
            .expect("in-memory build");
        assert_eq!(index.line_count(), 2);
        assert_index_finds_every_line(Charset::UTF_8, b"a\nb", &index);
    }

    /// `chunk_bytes` is caller-supplied and the chunk end is `from + chunk_bytes`. A size near
    /// `u64::MAX` passes both validation gates, so the addition has to saturate: it panics in debug
    /// and — worse — wraps to a `to` below `from` in release, silently dropping the chunk's lines.
    #[test]
    fn an_absurd_chunk_size_saturates_rather_than_overflowing() {
        let bytes = b"one\ntwo\nthree\n";
        let index = build_index(
            &bytes[..],
            Charset::UTF_8,
            0,
            bytes.len() as u64,
            &IndexOptions {
                stride: 4,
                chunk_bytes: u64::MAX - 1,
                read_bytes: 64,
                threads: 2,
            },
        )
        .expect("in-memory build");
        assert_eq!(index.line_count(), 3);
        assert_index_finds_every_line(Charset::UTF_8, bytes, &index);
    }

    #[test]
    fn an_empty_range_is_an_empty_index() {
        let index = build_index(&b""[..], Charset::UTF_8, 0, 0, &IndexOptions::default())
            .expect("in-memory build");
        assert!(index.is_empty());
        assert_eq!(index.anchor_at_or_before(0), None);
    }
}
