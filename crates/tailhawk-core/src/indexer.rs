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
use crate::index::{LineIndex, LineScanner, ANCHOR_STRIDE};
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
        .map(|from| (from, (from + opts.chunk_bytes).min(end)))
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
    for (nth, scan) in scans.iter_mut().enumerate() {
        let scan = scan
            .take()
            .ok_or_else(|| Error(format!("chunk {nth} was never scanned")))?;
        final_start = scan.last_start.or(final_start);
        index.append_chunk(scan.lines, &scan.anchors);
    }

    // A terminator as the file's last bytes opens a line start at end of file, and that is not a
    // line. `LineDecoder::finish` draws the same distinction, and the two must agree or the grid
    // and the index disagree about how many rows exist.
    if final_start == Some(end) {
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
    if owns_first_line {
        scan.lines = 1;
        scan.anchors.push(from);
        scan.last_start = Some(from);
    }

    let mut scanner = LineScanner::new(charset, from);
    let mut at = from;
    while at < to {
        let want = usize::try_from(to - at).unwrap_or(buf.len()).min(buf.len());
        let read = reader.read_at(at, &mut buf[..want])?;
        if read == 0 {
            // The file shrank under us. Rotation and truncation are M4's (§5.5); all this owes is
            // to stop rather than spin.
            break;
        }
        scanner.push(&buf[..read], |offset| {
            if scan.lines.is_multiple_of(stride) {
                scan.anchors.push(offset);
            }
            scan.lines += 1;
            scan.last_start = Some(offset);
        });
        at += read as u64;
    }

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

    #[test]
    fn an_empty_range_is_an_empty_index() {
        let index = build_index(&b""[..], Charset::UTF_8, 0, 0, &IndexOptions::default())
            .expect("in-memory build");
        assert!(index.is_empty());
        assert_eq!(index.anchor_at_or_before(0), None);
    }
}
