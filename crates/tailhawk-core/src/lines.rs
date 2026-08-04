//! Incremental line decoding — `SPEC.md` §5.6.
//!
//! One [`LineDecoder`] per followed source, **never reset between appends**. It owns two pieces of
//! carry that a naive "decode this chunk, split on newline" loop silently loses:
//!
//! 1. **A partial multi-byte sequence** at the end of a read. A 1 MB read boundary lands mid-glyph
//!    routinely, and resetting the decoder turns that glyph into two replacement characters
//!    forever.
//! 2. **A pending CR.** `CRLF` split across two reads is one terminator, not two, and the second
//!    read cannot tell on its own.
//!
//! Both are invisible in testing against ASCII fixtures read in one gulp, which is why the
//! boundary-invariance tests below feed every input at every possible split.

use crate::encoding::{Charset, Detection};
use encoding_rs::CoderResult;

/// Splits a byte stream into decoded lines, carrying partial state across chunk boundaries.
pub struct LineDecoder {
    decoder: DecoderImpl,
    /// Decoded text of the current chunk. Reused so that a steady tail does not allocate per read.
    scratch: String,
    /// The part of the current line that arrived in an earlier chunk.
    pending: String,
    /// The last line was terminated by a CR at the very end of a chunk. If the next chunk opens
    /// with LF, the two are one terminator.
    pending_cr: bool,
    /// Byte-order-mark bytes still to be swallowed. The BOM is consumed and never rendered, but it
    /// keeps its offsets in the index so byte offsets stay exact (§5.6).
    skip: usize,
}

impl LineDecoder {
    /// A decoder for the start of a file, which consumes the byte-order mark `detect` found.
    pub fn from_detection(detection: &Detection) -> Self {
        Self {
            decoder: DecoderImpl::new(detection.charset),
            scratch: String::new(),
            pending: String::new(),
            pending_cr: false,
            skip: detection.bom_len,
        }
    }

    /// A decoder for a read starting at a known code-unit-aligned offset — the short-lived kind
    /// §5.6 specifies for viewport reads, as opposed to the long-lived one following the tail.
    pub fn new(charset: Charset) -> Self {
        Self {
            decoder: DecoderImpl::new(charset),
            scratch: String::new(),
            pending: String::new(),
            pending_cr: false,
            skip: 0,
        }
    }

    /// Feeds one read's worth of bytes, calling `on_line` once per **complete** line.
    ///
    /// Line content excludes the terminator: `\r` is part of the terminator and is stripped, never
    /// treated as content (§5.6). A trailing partial line is held until it completes or until
    /// [`finish`](Self::finish).
    pub fn push(&mut self, bytes: &[u8], on_line: impl FnMut(&str)) {
        self.feed(bytes, false, on_line);
    }

    /// Signals end of stream: flushes any incomplete multi-byte sequence as U+FFFD and emits a
    /// final unterminated line if there is one.
    ///
    /// A file whose last line has no terminator still has that line. A file whose last byte *is* a
    /// terminator does not gain an empty one.
    pub fn finish(&mut self, on_line: impl FnMut(&str)) {
        self.feed(&[], true, on_line);
    }

    fn feed(&mut self, bytes: &[u8], last: bool, mut on_line: impl FnMut(&str)) {
        let mut bytes = bytes;
        if self.skip > 0 {
            let n = self.skip.min(bytes.len());
            bytes = &bytes[n..];
            self.skip -= n;
        }

        // Taken out so that `split` can hold `&mut self` while reading the decoded text.
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        self.decoder.decode_chunk(bytes, &mut scratch, last);
        self.split(&scratch, &mut on_line);
        self.scratch = scratch;

        if last && !self.pending.is_empty() {
            on_line(&self.pending);
            self.pending.clear();
        }
    }

    fn split(&mut self, text: &str, on_line: &mut impl FnMut(&str)) {
        let mut rest = text;

        // A CR ended the previous chunk. Only an LF *immediately* here belongs to it.
        //
        // The emptiness guard is load-bearing, and it is not obvious. A chunk can decode to no
        // characters at all — feeding UTF-16LE one byte at a time, every second read produces
        // nothing — and clearing the flag on such a chunk resolves the CR against a character that
        // has not arrived yet. The LF then turns up in the *next* chunk looking like a terminator
        // in its own right, and every CRLF in the file gains a phantom empty line after it.
        if self.pending_cr && !rest.is_empty() {
            self.pending_cr = false;
            if let Some(after) = rest.strip_prefix('\n') {
                rest = after;
            }
        }

        while let Some(at) = rest.find(['\r', '\n']) {
            let (line, from_terminator) = rest.split_at(at);

            if self.pending.is_empty() {
                on_line(line);
            } else {
                self.pending.push_str(line);
                on_line(&self.pending);
                self.pending.clear();
            }

            let terminator = from_terminator.as_bytes();
            rest = if terminator[0] == b'\r' {
                match terminator.get(1) {
                    Some(b'\n') => &from_terminator[2..],
                    // CR at the very end of the chunk: whether it is CR or CRLF is not knowable
                    // until the next read.
                    None => {
                        self.pending_cr = true;
                        return;
                    }
                    Some(_) => &from_terminator[1..],
                }
            } else {
                &from_terminator[1..]
            };
        }

        self.pending.push_str(rest);
    }
}

/// One-shot decode, for the plausibility check in [`crate::encoding`].
pub(crate) fn decode_all(charset: Charset, bytes: &[u8], dst: &mut String) {
    DecoderImpl::new(charset).decode_chunk(bytes, dst, true);
}

enum DecoderImpl {
    Whatwg(encoding_rs::Decoder),
    /// UTF-32 is outside the WHATWG Encoding Standard, so `encoding_rs` has no decoder for it.
    Utf32 {
        little_endian: bool,
        carry: [u8; 4],
        carry_len: usize,
    },
}

impl DecoderImpl {
    fn new(charset: Charset) -> Self {
        match charset {
            // BOM handling is ours: `detect` already identified and measured the mark, and
            // `LineDecoder::skip` consumes it. Letting the decoder sniff as well would either
            // double-consume or disagree.
            Charset::Whatwg(e) => DecoderImpl::Whatwg(e.new_decoder_without_bom_handling()),
            Charset::Utf32Le => DecoderImpl::Utf32 {
                little_endian: true,
                carry: [0; 4],
                carry_len: 0,
            },
            Charset::Utf32Be => DecoderImpl::Utf32 {
                little_endian: false,
                carry: [0; 4],
                carry_len: 0,
            },
        }
    }

    /// Decodes with U+FFFD replacement, never erroring (§5.6). Malformed bytes are a property of
    /// real logs — a truncated write, a binary blob, a wrong guess — and refusing to display the
    /// file is never the right answer.
    fn decode_chunk(&mut self, src: &[u8], dst: &mut String, last: bool) {
        match self {
            DecoderImpl::Whatwg(decoder) => {
                let mut pos = 0;
                loop {
                    let remaining = &src[pos..];
                    let needed = decoder
                        .max_utf8_buffer_length(remaining.len())
                        .unwrap_or(remaining.len().saturating_mul(3) + 4);
                    dst.reserve(needed.max(4));

                    let (result, read, _) = decoder.decode_to_string(remaining, dst, last);
                    pos += read;
                    match result {
                        CoderResult::InputEmpty => break,
                        // `max_utf8_buffer_length` is a guarantee, so this is unreachable in
                        // practice; going round again with more space is still the only correct
                        // response to it.
                        CoderResult::OutputFull => dst.reserve(needed.max(1024)),
                    }
                }
            }

            DecoderImpl::Utf32 {
                little_endian,
                carry,
                carry_len,
            } => {
                let little_endian = *little_endian;
                let mut i = 0;

                while *carry_len > 0 && i < src.len() {
                    carry[*carry_len] = src[i];
                    *carry_len += 1;
                    i += 1;
                    if *carry_len == 4 {
                        push_scalar(*carry, little_endian, dst);
                        *carry_len = 0;
                    }
                }

                // Only if the carry was completed. A still-partial carry means `src` ran out
                // inside it, so there is no tail to record — and recording one here would
                // overwrite `carry_len` with zero and throw the carry away. That is the whole bug
                // this reads as a guard against: it makes 1-byte reads of a UTF-32 file decode to
                // nothing at all, forever.
                if *carry_len == 0 {
                    while src.len() - i >= 4 {
                        let unit = [src[i], src[i + 1], src[i + 2], src[i + 3]];
                        push_scalar(unit, little_endian, dst);
                        i += 4;
                    }

                    let tail = &src[i..];
                    carry[..tail.len()].copy_from_slice(tail);
                    *carry_len = tail.len();
                }

                if last && *carry_len > 0 {
                    dst.push(char::REPLACEMENT_CHARACTER);
                    *carry_len = 0;
                }
            }
        }
    }
}

/// A UTF-32 code unit is a scalar value or it is nothing. Surrogates are unpaired by construction
/// here, so `char::from_u32` rejecting them is the correct behaviour, not a limitation.
fn push_scalar(unit: [u8; 4], little_endian: bool, dst: &mut String) {
    let value = if little_endian {
        u32::from_le_bytes(unit)
    } else {
        u32::from_be_bytes(unit)
    };
    dst.push(char::from_u32(value).unwrap_or(char::REPLACEMENT_CHARACTER));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::{detect, Confidence, Detection};

    fn collect(charset: Charset, bytes: &[u8], chunk: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut d = LineDecoder::new(charset);
        for part in bytes.chunks(chunk.max(1)) {
            d.push(part, |line| out.push(line.to_owned()));
        }
        d.finish(|line| out.push(line.to_owned()));
        out
    }

    fn all_at_once(charset: Charset, bytes: &[u8]) -> Vec<String> {
        collect(charset, bytes, bytes.len().max(1))
    }

    /// The property M1 exists to guarantee: **where the reads happen cannot change the lines**.
    ///
    /// This is the test that catches both kinds of carry at once, and it is the reason it feeds
    /// every split rather than a plausible-looking one. A decoder that resets per chunk passes at
    /// chunk sizes that happen to land on boundaries and fails at the ones that do not.
    fn boundaries_are_invisible(charset: Charset, bytes: &[u8]) {
        let expected = all_at_once(charset, bytes);
        for chunk in 1..=bytes.len().min(40) {
            assert_eq!(
                collect(charset, bytes, chunk),
                expected,
                "{} split into {chunk}-byte reads produced different lines",
                charset.name()
            );
        }
        // A couple of sizes that are coprime with every code-unit width, well past the small end.
        for chunk in [7, 13, 31, 63, 101] {
            assert_eq!(
                collect(charset, bytes, chunk),
                expected,
                "{} split into {chunk}-byte reads produced different lines",
                charset.name()
            );
        }
    }

    fn utf16le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }
    fn utf32le(s: &str) -> Vec<u8> {
        s.chars().flat_map(|c| (c as u32).to_le_bytes()).collect()
    }

    // --- Terminators ------------------------------------------------------------------------

    #[test]
    fn all_three_terminators_and_a_mixture() {
        // §5.6: the set is {CRLF, LF, CR}, and a whole-file "this is a CRLF file" decision is
        // wrong because mixed files are routine.
        let input = b"crlf\r\nlf\ncr\rmixed\r\nagain\n";
        assert_eq!(
            all_at_once(Charset::UTF_8, input),
            ["crlf", "lf", "cr", "mixed", "again"]
        );
    }

    #[test]
    fn cr_is_terminator_not_content() {
        for line in all_at_once(Charset::UTF_8, b"a\r\nb\r\n") {
            assert!(!line.contains('\r'), "{line:?} kept its terminator");
        }
    }

    #[test]
    fn empty_lines_are_preserved() {
        assert_eq!(
            all_at_once(Charset::UTF_8, b"a\n\nb\r\n\r\nc"),
            ["a", "", "b", "", "c"]
        );
    }

    #[test]
    fn consecutive_bare_crs_are_separate_terminators() {
        assert_eq!(all_at_once(Charset::UTF_8, b"a\r\rb"), ["a", "", "b"]);
    }

    #[test]
    fn a_trailing_terminator_does_not_invent_an_empty_line() {
        assert_eq!(all_at_once(Charset::UTF_8, b"a\nb\n"), ["a", "b"]);
        assert_eq!(all_at_once(Charset::UTF_8, b"a\nb\r\n"), ["a", "b"]);
    }

    #[test]
    fn an_unterminated_final_line_is_still_a_line() {
        assert_eq!(all_at_once(Charset::UTF_8, b"a\nb"), ["a", "b"]);
    }

    #[test]
    fn an_empty_stream_has_no_lines() {
        assert!(all_at_once(Charset::UTF_8, b"").is_empty());
    }

    /// The specific carry a per-chunk decoder loses: `\r` ends read *n* and `\n` starts read
    /// *n+1*. Handled wrongly this doubles every line count in a CRLF file read at the wrong
    /// boundary, and the extra lines are empty, so it looks like double-spacing rather than a bug.
    #[test]
    fn crlf_split_across_a_read_boundary_is_one_terminator() {
        let mut out = Vec::new();
        let mut d = LineDecoder::new(Charset::UTF_8);
        d.push(b"first\r", |l| out.push(l.to_owned()));
        d.push(b"\nsecond\r\n", |l| out.push(l.to_owned()));
        d.finish(|l| out.push(l.to_owned()));
        assert_eq!(out, ["first", "second"]);
    }

    /// The opposite mistake: swallowing an LF that is a terminator in its own right because a CR
    /// happened to end the previous chunk *and* an empty line follows.
    #[test]
    fn a_pending_cr_only_swallows_an_immediately_following_lf() {
        let mut out = Vec::new();
        let mut d = LineDecoder::new(Charset::UTF_8);
        d.push(b"a\r", |l| out.push(l.to_owned()));
        d.push(b"b\n", |l| out.push(l.to_owned()));
        d.finish(|l| out.push(l.to_owned()));
        assert_eq!(out, ["a", "b"]);
    }

    // --- Carry across boundaries, per encoding -----------------------------------------------

    #[test]
    fn utf8_boundaries_are_invisible() {
        let text = "ascii line\r\nem — dash\nCJK 日本語のログ\r\nemoji 🦅 tail\nlast";
        boundaries_are_invisible(Charset::UTF_8, text.as_bytes());
    }

    #[test]
    fn utf16le_boundaries_are_invisible() {
        // Includes a surrogate pair, so a 2-byte-aligned split can still land mid-character.
        let text = "ascii\r\nem — dash\nemoji 🦅\r\nlast";
        boundaries_are_invisible(Charset::UTF_16LE, &utf16le(text));
    }

    #[test]
    fn utf32le_boundaries_are_invisible() {
        let text = "ascii\r\nem — dash\nemoji 🦅\r\nlast";
        boundaries_are_invisible(Charset::Utf32Le, &utf32le(text));
    }

    #[test]
    fn shift_jis_boundaries_are_invisible() {
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode("2026-07-31 情報\r\nワーカー開始\n終了");
        boundaries_are_invisible(Charset::Whatwg(encoding_rs::SHIFT_JIS), &bytes);
    }

    #[test]
    fn a_multibyte_sequence_split_across_reads_survives_intact() {
        let bytes = "—".as_bytes();
        assert_eq!(bytes.len(), 3);
        let mut out = Vec::new();
        let mut d = LineDecoder::new(Charset::UTF_8);
        d.push(&bytes[..1], |l| out.push(l.to_owned()));
        d.push(&bytes[1..2], |l| out.push(l.to_owned()));
        d.push(&bytes[2..], |l| out.push(l.to_owned()));
        d.finish(|l| out.push(l.to_owned()));
        assert_eq!(out, ["—"], "the decoder was reset between reads");
    }

    // --- Malformed input ----------------------------------------------------------------------

    #[test]
    fn a_sequence_truncated_at_eof_becomes_one_replacement_and_does_not_panic() {
        let mut bytes = b"good line\n".to_vec();
        bytes.extend_from_slice(&"—".as_bytes()[..2]);
        assert_eq!(
            all_at_once(Charset::UTF_8, &bytes),
            ["good line", "\u{FFFD}"]
        );
    }

    #[test]
    fn utf32_truncated_at_eof_becomes_one_replacement() {
        let mut bytes = utf32le("ok\n");
        bytes.push(0x41);
        assert_eq!(all_at_once(Charset::Utf32Le, &bytes), ["ok", "\u{FFFD}"]);
    }

    /// A UTF-32 code unit above U+10FFFF, and one in the surrogate range, are both "not a scalar
    /// value". Neither may abort the read.
    #[test]
    fn out_of_range_utf32_units_become_replacements() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&0x0011_0000u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000_D800u32.to_le_bytes());
        bytes.extend_from_slice(&(b'x' as u32).to_le_bytes());
        assert_eq!(all_at_once(Charset::Utf32Le, &bytes), ["\u{FFFD}\u{FFFD}x"]);
    }

    #[test]
    fn embedded_nuls_and_control_bytes_are_never_silently_dropped() {
        // §5.6: control bytes and NULs survive decoding; rendering them as a visible replacement
        // glyph is the grid's job (§13.4), not this layer's.
        let lines = all_at_once(Charset::UTF_8, b"before\x00\x07after\n");
        assert_eq!(lines, ["before\u{0}\u{7}after"]);
    }

    // --- The BOM ------------------------------------------------------------------------------

    #[test]
    fn the_bom_is_consumed_and_never_appears_in_a_line() {
        for (bom, charset, body) in [
            (
                vec![0xEF, 0xBB, 0xBF],
                Charset::UTF_8,
                "first\nsecond".as_bytes().to_vec(),
            ),
            (
                vec![0xFF, 0xFE],
                Charset::UTF_16LE,
                utf16le("first\nsecond"),
            ),
            (
                vec![0xFF, 0xFE, 0x00, 0x00],
                Charset::Utf32Le,
                utf32le("first\nsecond"),
            ),
        ] {
            let mut bytes = bom.clone();
            bytes.extend_from_slice(&body);
            let detection = Detection {
                charset,
                bom_len: bom.len(),
                confidence: Confidence::Bom,
                disagreed: false,
            };

            for chunk in 1..=8 {
                let mut out = Vec::new();
                let mut d = LineDecoder::from_detection(&detection);
                for part in bytes.chunks(chunk) {
                    d.push(part, |l| out.push(l.to_owned()));
                }
                d.finish(|l| out.push(l.to_owned()));
                assert_eq!(
                    out,
                    ["first", "second"],
                    "{} with a {}-byte BOM, read {chunk} bytes at a time",
                    charset.name(),
                    bom.len()
                );
            }
        }
    }

    /// End to end, the way a source will use it: detect, then decode with the detection.
    #[test]
    fn detection_and_decoding_compose() {
        let text = "2026-07-31 09:14:14  [worker]  stage — complete\r\n17:00:02\tINSTANCE\tdone\n";
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&utf16le(text));

        let detection = detect(&bytes, None, encoding_rs::WINDOWS_1252);
        assert_eq!(detection.charset, Charset::UTF_16LE);

        let mut out = Vec::new();
        let mut d = LineDecoder::from_detection(&detection);
        for part in bytes.chunks(5) {
            d.push(part, |l| out.push(l.to_owned()));
        }
        d.finish(|l| out.push(l.to_owned()));

        assert_eq!(
            out,
            [
                "2026-07-31 09:14:14  [worker]  stage — complete",
                "17:00:02\tINSTANCE\tdone"
            ]
        );
    }

    /// xorshift64*, so the fuzz loop below needs no dependency and therefore no `deny.toml`
    /// review. A fuzzer needs a *reproducible* seed far more than it needs a good distribution.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// Uniform random bytes almost never produce a `CRLF` pair, a valid multi-byte sequence or a
    /// BOM, so a uniform fuzzer explores none of the states that actually carry across chunks.
    /// Drawing most bytes from this alphabet is what makes the loop hit them.
    const INTERESTING: &[u8] = &[
        0x00, 0x09, 0x0A, 0x0D, 0x1B, 0x20, 0x41, 0x7F, 0x80, 0xBF, 0xC2, 0xE2, 0xEF, 0xBB, 0xBF,
        0xF0, 0xF4, 0xFE, 0xFF,
    ];

    fn arbitrary_bytes(rng: &mut Rng) -> Vec<u8> {
        let len = rng.below(129);
        (0..len)
            .map(|_| {
                if rng.below(4) == 0 {
                    rng.below(256) as u8
                } else {
                    INTERESTING[rng.below(INTERESTING.len())]
                }
            })
            .collect()
    }

    fn decode_all(charset: Charset, bytes: &[u8], chunk: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut d = LineDecoder::new(charset);
        for part in bytes.chunks(chunk.max(1)) {
            d.push(part, |l| out.push(l.to_owned()));
        }
        d.finish(|l| out.push(l.to_owned()));
        out
    }

    /// The fuzz target `PLAN.md` §4 asks of M1, as an ordinary test rather than under `cargo-fuzz`.
    ///
    /// **Why not `cargo-fuzz`:** it requires libFuzzer, which is poorly supported on MSVC and
    /// effectively absent on Windows ARM64 — and CI builds both. An ordinary `#[test]` runs in the
    /// existing job on both architectures with no new tooling, and a fixed iteration count keeps CI
    /// deterministic in a way a wall-clock budget would not. Raise `TAILHAWK_FUZZ_ITERS` for a
    /// longer local soak.
    ///
    /// The boundary-invariance tests above feed *fixed* inputs at every split; this feeds
    /// *arbitrary* content at several splits. Three properties, none of which may depend on how the
    /// bytes were framed:
    ///
    /// 1. No input panics the decoder.
    /// 2. No emitted line contains `\r` or `\n` — a terminator must never survive as content.
    /// 3. The lines are identical however the same bytes are chunked, including invalid ones.
    ///    Replacement characters are part of the output and must land in the same places.
    #[test]
    fn arbitrary_bytes_decode_the_same_however_they_are_chunked() {
        let charsets = [
            Charset::UTF_8,
            Charset::UTF_16LE,
            Charset::UTF_16BE,
            Charset::Utf32Le,
            Charset::Utf32Be,
            Charset::Whatwg(encoding_rs::WINDOWS_1252),
            Charset::Whatwg(encoding_rs::SHIFT_JIS),
            Charset::Whatwg(encoding_rs::GBK),
            Charset::Whatwg(encoding_rs::BIG5),
            Charset::Whatwg(encoding_rs::EUC_KR),
            Charset::Whatwg(encoding_rs::ISO_2022_JP),
        ];

        let iters: usize = std::env::var("TAILHAWK_FUZZ_ITERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(400);

        let mut seeds = Rng(0x7a11_4841_574b_0001);
        for _ in 0..iters {
            let seed = seeds.next();
            let bytes = arbitrary_bytes(&mut Rng(seed));

            for charset in charsets {
                let reference = decode_all(charset, &bytes, bytes.len().max(1));

                for line in &reference {
                    assert!(
                        !line.contains(['\n', '\r']),
                        "{} kept a terminator as content: {line:?}, seed {seed:#018x}, bytes {bytes:02x?}",
                        charset.name()
                    );
                }

                for chunk in [1, 2, 3, 5, 7, 13] {
                    let split = decode_all(charset, &bytes, chunk);
                    assert_eq!(
                        split,
                        reference,
                        "{} framed {chunk} bytes at a time disagrees with one gulp, \
                         seed {seed:#018x}, bytes {bytes:02x?}",
                        charset.name()
                    );
                }
            }
        }
    }
}
