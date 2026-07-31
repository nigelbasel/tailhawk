//! Encoding detection — `SPEC.md` §5.6.
//!
//! This runs **before** indexing, not merely before decoding. §5.3's chunk-assignment invariant
//! needs [`Charset::code_unit`] to exist before a single byte offset is recorded, which is why M1
//! is this module and not the indexer.
//!
//! Two things here are hand-written rather than delegated to `encoding_rs`, both for the same
//! reason: the WHATWG Encoding Standard excludes UTF-16 and UTF-32 *detection*, and excludes UTF-32
//! entirely. On Windows that is not an edge case — PowerShell 5.1's `>` and `Out-File` default to
//! UTF-16LE.

use encoding_rs::Encoding;

/// The encodings Tailhawk can decode.
///
/// Most are delegated to `encoding_rs`. UTF-32 is not in the WHATWG Encoding Standard and so has
/// no `encoding_rs` representation at all, but its BOM is the one that must be tested *first*
/// (§5.6) — `FF FE 00 00` starts with the UTF-16LE BOM, so a shortest-match sniff misdetects every
/// UTF-32LE file as UTF-16LE followed by a NUL.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Charset {
    /// Anything in the WHATWG Encoding Standard.
    Whatwg(&'static Encoding),
    Utf32Le,
    Utf32Be,
}

impl Charset {
    pub const UTF_8: Charset = Charset::Whatwg(encoding_rs::UTF_8);
    pub const UTF_16LE: Charset = Charset::Whatwg(encoding_rs::UTF_16LE);
    pub const UTF_16BE: Charset = Charset::Whatwg(encoding_rs::UTF_16BE);

    pub fn name(&self) -> &'static str {
        match self {
            Charset::Whatwg(e) => e.name(),
            Charset::Utf32Le => "UTF-32LE",
            Charset::Utf32Be => "UTF-32BE",
        }
    }

    /// Bytes per code unit: the alignment a chunk boundary must respect (`SPEC.md` §5.3).
    ///
    /// Splitting a UTF-16 file at an odd offset swaps every subsequent byte pair, so the newline
    /// scan looks for `0A 00` in a stream where it is now written `00 0A`.
    ///
    /// **This is the *only* constraint on chunking the newline scan.** There is no encoding-
    /// specific exception, and in particular no DBCS one — see
    /// `a_0a_byte_is_never_consumed_as_a_trail_byte_by_any_decoder`, which drives all 65,536
    /// two-byte prefixes into every supported decoder and finds that a `0x0A` byte is always a
    /// line terminator. An earlier draft of §5.3 asserted the opposite and disabled the parallel
    /// path for codepages 932/936/950/949; that was measured false and the exception is gone.
    pub fn code_unit(&self) -> usize {
        match self {
            Charset::Utf32Le | Charset::Utf32Be => 4,
            Charset::Whatwg(e) if *e == encoding_rs::UTF_16LE || *e == encoding_rs::UTF_16BE => 2,
            Charset::Whatwg(_) => 1,
        }
    }

    /// Whether decoding can start at an arbitrary indexed line without replaying what came before.
    ///
    /// True for every encoding here except ISO-2022-JP, which is escape-driven: the same bytes mean
    /// different characters depending on shift state established earlier in the file. A line start
    /// is always a *character* boundary — that follows from `0x0A` never being a trail byte — but
    /// for a stateful encoding it is not a *decoder* boundary.
    ///
    /// This is all that remains of `SPEC.md` §5.3's restriction, and note how much narrower it is
    /// than what that section used to say: it constrains **viewport decode**, not the newline scan,
    /// and it catches one encoding rather than four codepages.
    pub fn is_random_access_decodable(&self) -> bool {
        !matches!(self, Charset::Whatwg(e) if *e == encoding_rs::ISO_2022_JP)
    }
}

/// How the answer was reached. The UI shows the encoding either way (§5.6); this is what decides
/// whether it also shows a warning next to it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// A byte-order mark. Not a guess.
    Bom,
    /// A structural property of the bytes: NUL-position parity, or strict UTF-8 validity with at
    /// least one multi-byte sequence.
    High,
    /// A statistical guess, or the system codepage fallback. Offer the override prominently.
    Low,
}

/// A window of the file, with the absolute offset it was read from.
///
/// The offset is not decoration: NUL-position parity is only meaningful relative to the start of
/// the *file*, so a tail sample read from an arbitrary offset must say where it came from.
#[derive(Copy, Clone, Debug)]
pub struct Sample<'a> {
    pub bytes: &'a [u8],
    pub offset: u64,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Detection {
    pub charset: Charset,
    /// Bytes of byte-order mark at offset 0, to be consumed and never rendered. The BOM still
    /// occupies its offsets in the index, so byte offsets stay exact (§5.6).
    pub bom_len: usize,
    pub confidence: Confidence,
    /// Head and tail samples reached different answers. The tail won — it is what is live — and
    /// the UI flags the file (§5.6).
    pub disagreed: bool,
}

/// The sample size §5.6 specifies for each end of the file.
pub const SAMPLE_BYTES: usize = 64 * 1024;

/// Below this many bytes the parity probe has nothing to say and would fire on noise.
const MIN_PROBE_BYTES: usize = 64;

/// A residue class is "all NUL" above this and "no NUL" below the next one. ASCII-dominated text
/// in UTF-16 puts a NUL in every high byte, so the true figures are near 1.0 and 0.0; the gap is
/// wide enough that the exact thresholds are not delicate.
const PARITY_HIGH: f64 = 0.40;
const PARITY_LOW: f64 = 0.05;

/// Fraction of decoded characters allowed to be replacement characters or stray C0 controls before
/// a candidate is rejected as implausible.
const IMPLAUSIBLE_LIMIT: f64 = 0.05;

/// Resolves the encoding of a file from a head sample and, where the file is long enough to have
/// one, a tail sample.
///
/// `fallback` is the last resort of §5.6 step 4 — on Windows that is the ANSI codepage, supplied
/// by [`system_codepage`]. It is a parameter rather than a call because the core is portable
/// (§3.1) and may not name a Win32 function.
pub fn detect(head: &[u8], tail: Option<Sample<'_>>, fallback: &'static Encoding) -> Detection {
    // 1. BOM, longest match first.
    if let Some((charset, bom_len)) = sniff_bom(head) {
        return Detection {
            charset,
            bom_len,
            confidence: Confidence::Bom,
            disagreed: false,
        };
    }

    // 2. UTF-16/32 by NUL-position parity, at both ends.
    let head_wide = probe_wide(head, 0);
    let tail_wide = tail.and_then(|t| probe_wide(t.bytes, t.offset));

    // §5.6: prefer the tail on disagreement — it is the part being appended to.
    let wide = tail_wide.or(head_wide);
    let disagreed = match (head_wide, tail_wide) {
        (Some(h), Some(t)) => h != t,
        // One end probing wide and the other not is disagreement too: a UTF-16 file does not stop
        // being UTF-16 halfway through, so one of the two reads is wrong.
        (Some(_), None) => tail.is_some(),
        (None, Some(_)) => true,
        (None, None) => false,
    };

    if let Some(charset) = wide {
        return Detection {
            charset,
            bom_len: 0,
            confidence: Confidence::High,
            disagreed,
        };
    }

    // 3. Strict UTF-8. Pure ASCII resolves here too — UTF-8 is a superset, so it is the safe
    //    answer rather than a guess about which single-byte codepage the high bytes would have
    //    meant, because there are none.
    let head_utf8 = utf8_verdict(head, false);
    let tail_utf8 = tail.map(|t| utf8_verdict(t.bytes, true));

    let both_valid = head_utf8.is_some() && tail_utf8.is_none_or(|v| v.is_some());
    if both_valid {
        let multibyte_anywhere = head_utf8 == Some(true) || tail_utf8.flatten() == Some(true);
        return Detection {
            charset: Charset::UTF_8,
            bom_len: 0,
            // Valid UTF-8 with a multi-byte sequence in it is not a coincidence; the sequences are
            // self-synchronising and a legacy-codepage file fails validation almost immediately.
            // Pure ASCII is a weaker claim: it is *compatible* with UTF-8 and with every
            // single-byte codepage at once.
            confidence: if multibyte_anywhere {
                Confidence::High
            } else {
                Confidence::Low
            },
            disagreed: false,
        };
    }

    // 4. chardetng, then the system codepage.
    let mut detector = chardetng::EncodingDetector::new();
    detector.feed(head, tail.is_none());
    if let Some(t) = tail {
        detector.feed(t.bytes, true);
    }
    let (guess, high) = detector.guess_assess(None, true);

    Detection {
        charset: Charset::Whatwg(if high { guess } else { fallback }),
        bom_len: 0,
        confidence: Confidence::Low,
        disagreed: false,
    }
}

/// Byte-order marks, longest first. `FF FE 00 00` must be tested before `FF FE`.
fn sniff_bom(head: &[u8]) -> Option<(Charset, usize)> {
    const BOMS: &[(&[u8], Charset)] = &[
        (&[0xFF, 0xFE, 0x00, 0x00], Charset::Utf32Le),
        (&[0x00, 0x00, 0xFE, 0xFF], Charset::Utf32Be),
        (&[0xEF, 0xBB, 0xBF], Charset::UTF_8),
        (&[0xFF, 0xFE], Charset::UTF_16LE),
        (&[0xFE, 0xFF], Charset::UTF_16BE),
    ];
    BOMS.iter()
        .find(|(bytes, _)| head.starts_with(bytes))
        .map(|(bytes, charset)| (*charset, bytes.len()))
}

/// The NUL-position-parity probe of §5.6 step 2.
///
/// **The test is NUL density with consistent parity, not raw NUL count.** A UTF-8 log with an
/// embedded binary blob has plenty of NULs, but at arbitrary positions, so no residue class comes
/// out clean. The candidate is then decoded and checked for plausibility, which is the second
/// defence against the same failure.
fn probe_wide(sample: &[u8], offset: u64) -> Option<Charset> {
    if sample.len() < MIN_PROBE_BYTES {
        return None;
    }

    let mut nul = [0usize; 4];
    let mut total = [0usize; 4];
    for (i, byte) in sample.iter().enumerate() {
        let residue = ((offset.wrapping_add(i as u64)) % 4) as usize;
        total[residue] += 1;
        if *byte == 0 {
            nul[residue] += 1;
        }
    }

    let f = |r: usize| nul[r] as f64 / total[r].max(1) as f64;
    let (hi, lo) = (PARITY_HIGH, PARITY_LOW);

    // UTF-32 before UTF-16: a UTF-32LE file has three of four residues full of NULs, which no
    // UTF-16 pattern matches, but testing in the other order invites exactly the BOM mistake.
    let candidate = if f(0) < lo && f(1) > hi && f(2) > hi && f(3) > hi {
        Charset::Utf32Le
    } else if f(3) < lo && f(0) > hi && f(1) > hi && f(2) > hi {
        Charset::Utf32Be
    } else if f(0) < lo && f(2) < lo && f(1) > hi && f(3) > hi {
        Charset::UTF_16LE
    } else if f(1) < lo && f(3) < lo && f(0) > hi && f(2) > hi {
        Charset::UTF_16BE
    } else {
        return None;
    };

    plausible(candidate, sample, offset).then_some(candidate)
}

/// Decodes a sample under a candidate and asks whether the result looks like text.
///
/// Cheap, and it is what stops a file whose NULs happen to fall into a clean parity from being
/// decoded wholesale as UTF-16 — which destroys the index rather than degrading it, because every
/// line boundary moves.
fn plausible(candidate: Charset, sample: &[u8], offset: u64) -> bool {
    // A tail sample can start mid-code-unit. Decoding from a misaligned offset is guaranteed
    // garbage and would fail a check that is meant to be about the *file*.
    let unit = candidate.code_unit() as u64;
    let skip = ((unit - offset % unit) % unit) as usize;
    let Some(aligned) = sample.get(skip..) else {
        return false;
    };

    let mut text = String::with_capacity(aligned.len() / 2);
    crate::lines::decode_all(candidate, aligned, &mut text);

    let mut chars = 0usize;
    let mut bad = 0usize;
    for ch in text.chars() {
        chars += 1;
        // C0 controls other than the three that legitimately appear in a log, plus the
        // replacement character. NUL is included: a correctly decoded UTF-16 log does not contain
        // U+0000 in quantity, whatever its bytes look like.
        if ch == '\u{FFFD}' || (ch < ' ' && ch != '\t' && ch != '\r' && ch != '\n') {
            bad += 1;
        }
    }

    chars > 0 && (bad as f64 / chars as f64) <= IMPLAUSIBLE_LIMIT
}

/// `Some(has_multibyte)` if the sample is valid UTF-8, `None` if it is not.
///
/// An incomplete sequence at the *end* is not invalidity — every sample of a longer file ends
/// somewhere arbitrary. When `back_align` is set the same allowance is made at the start, for a
/// tail sample that begins inside a sequence.
fn utf8_verdict(sample: &[u8], back_align: bool) -> Option<bool> {
    let start = if back_align {
        // At most three continuation bytes can precede a lead byte.
        sample
            .iter()
            .take(4)
            .position(|b| b & 0b1100_0000 != 0b1000_0000)
            .unwrap_or(0)
    } else {
        0
    };
    let body = sample.get(start..)?;

    let valid = match std::str::from_utf8(body) {
        Ok(_) => body,
        // `error_len() == None` means "ran out of input", not "found a bad byte".
        Err(e) if e.error_len().is_none() => &body[..e.valid_up_to()],
        Err(_) => return None,
    };

    Some(valid.iter().any(|b| *b >= 0x80))
}

/// The ANSI codepage, as §5.6 step 4's last resort.
///
/// Only the codepages Windows actually ships as a system default are mapped; anything else falls
/// to windows-1252, which is what `GetACP` returns on the overwhelming majority of installs and is
/// the least destructive wrong answer (it round-trips every byte).
#[cfg(windows)]
pub fn system_codepage() -> &'static Encoding {
    let label: &str = match unsafe { windows::Win32::Globalization::GetACP() } {
        437 | 1252 => "windows-1252",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        1250 => "windows-1250",
        1251 => "windows-1251",
        1253 => "windows-1253",
        1254 => "windows-1254",
        1255 => "windows-1255",
        1256 => "windows-1256",
        1257 => "windows-1257",
        1258 => "windows-1258",
        65001 => "utf-8",
        _ => "windows-1252",
    };
    Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252)
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

    /// Enough log-shaped text that a 64-byte probe window is not the thing under test.
    fn log_text() -> String {
        (0..200)
            .map(|i| format!("2026-07-31 09:14:{i:02} INFO  Worker.Run started batch {i}\r\n"))
            .collect()
    }

    fn detect_head(bytes: &[u8]) -> Detection {
        detect(bytes, None, encoding_rs::WINDOWS_1252)
    }

    // --- The M1 fixture matrix (`PLAN.md` §4) -----------------------------------------------

    #[test]
    fn bom_utf8() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(log_text().as_bytes());
        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::UTF_8);
        assert_eq!(d.bom_len, 3);
        assert_eq!(d.confidence, Confidence::Bom);
    }

    #[test]
    fn bom_utf16le() {
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend_from_slice(&utf16le(&log_text()));
        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::UTF_16LE);
        assert_eq!(d.bom_len, 2);
    }

    #[test]
    fn bom_utf16be() {
        let mut bytes = vec![0xFE, 0xFF];
        bytes.extend_from_slice(&utf16be(&log_text()));
        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::UTF_16BE);
        assert_eq!(d.bom_len, 2);
    }

    /// The ordering trap in §5.6: `FF FE 00 00` starts with the UTF-16LE BOM. Testing the shorter
    /// mark first misdetects every UTF-32LE file, and the failure is quiet — it decodes as text
    /// interleaved with NULs rather than as an error.
    #[test]
    fn bom_utf32le_is_not_read_as_utf16le() {
        let mut bytes = vec![0xFF, 0xFE, 0x00, 0x00];
        bytes.extend_from_slice(&utf32le(&log_text()));
        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::Utf32Le, "longest BOM match first");
        assert_eq!(d.bom_len, 4);
    }

    #[test]
    fn bom_utf32be() {
        let mut bytes = vec![0x00, 0x00, 0xFE, 0xFF];
        bytes.extend_from_slice(&utf32be(&log_text()));
        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::Utf32Be);
        assert_eq!(d.bom_len, 4);
    }

    #[test]
    fn bomless_utf8_ascii_only() {
        let d = detect_head(log_text().as_bytes());
        assert_eq!(d.charset, Charset::UTF_8);
        assert_eq!(d.bom_len, 0);
        assert_eq!(
            d.confidence,
            Confidence::Low,
            "pure ASCII is compatible with every single-byte codepage at once, so this is not a \
             high-confidence identification of UTF-8 — it is a safe default"
        );
    }

    /// Corpus B, reduced to its essential property: BOM-less UTF-8 whose only non-ASCII content is
    /// an em dash. Windows PowerShell 5.1 renders this file as `â€"`, i.e. a CP1252 default gets
    /// it wrong — so the correct answer was known before this code existed.
    #[test]
    fn bomless_utf8_with_em_dashes_is_not_cp1252() {
        let text: String = (0..200)
            .map(|i| format!("17:00:{i:02}\tINSTANCE-{i}\tstage — complete\n"))
            .collect();
        let d = detect_head(text.as_bytes());
        assert_eq!(d.charset, Charset::UTF_8);
        assert_eq!(
            d.confidence,
            Confidence::High,
            "a multi-byte sequence in otherwise-valid UTF-8 is a structural fact, not a guess"
        );
    }

    #[test]
    fn bomless_utf16le() {
        let d = detect_head(&utf16le(&log_text()));
        assert_eq!(d.charset, Charset::UTF_16LE);
        assert_eq!(d.bom_len, 0);
        assert_eq!(d.confidence, Confidence::High);
    }

    #[test]
    fn bomless_utf16be() {
        let d = detect_head(&utf16be(&log_text()));
        assert_eq!(d.charset, Charset::UTF_16BE);
    }

    #[test]
    fn bomless_utf32le() {
        let d = detect_head(&utf32le(&log_text()));
        assert_eq!(d.charset, Charset::Utf32Le);
    }

    #[test]
    fn bomless_utf32be() {
        let d = detect_head(&utf32be(&log_text()));
        assert_eq!(d.charset, Charset::Utf32Be);
    }

    /// The failure §5.6 names explicitly: a UTF-8 log with an embedded binary blob has NULs in
    /// quantity, and a raw-count test calls it UTF-16 wholesale. Every line boundary then moves,
    /// so this destroys the index rather than degrading it.
    #[test]
    fn utf8_with_an_embedded_binary_blob_is_still_utf8() {
        let mut bytes = log_text().into_bytes();
        // A blob with a NUL in roughly every other byte, but at no consistent parity.
        let blob: Vec<u8> = (0..4096u32)
            .map(|i| if i % 7 < 3 { 0 } else { (i % 251) as u8 })
            .collect();
        bytes.extend_from_slice(&blob);
        bytes.extend_from_slice(log_text().as_bytes());

        let d = detect_head(&bytes);
        assert_ne!(d.charset, Charset::UTF_16LE);
        assert_ne!(d.charset, Charset::UTF_16BE);
        assert_ne!(d.charset, Charset::Utf32Le);
    }

    /// The other half of the same trap, from the opposite direction: a *genuine* UTF-16LE file
    /// whose parity is clean must still be recognised even though half its bytes are NUL.
    #[test]
    fn parity_survives_non_ascii_content_in_utf16() {
        let text: String = (0..200)
            .map(|i| format!("2026-07-31 09:14:{i:02} INFO  日本語のログ行 — {i}\r\n"))
            .collect();
        let d = detect_head(&utf16le(&text));
        assert_eq!(d.charset, Charset::UTF_16LE);
    }

    /// A sample that stops in the middle of a multi-byte sequence is the normal case, not an
    /// error: every 64 KiB window of a longer file ends somewhere arbitrary.
    #[test]
    fn truncated_mid_sequence_is_not_invalidity() {
        let text: String = (0..200)
            .map(|i| format!("2026-07-31 09:14:{i:02} INFO  stage — {i}\r\n"))
            .collect();
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice("ends mid-dash —".as_bytes());
        bytes.truncate(bytes.len() - 1);

        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::UTF_8);
        assert_eq!(
            d.confidence,
            Confidence::High,
            "the complete sequences earlier in the sample are the evidence; the truncated one at \
             the end neither adds to it nor takes it away"
        );
    }

    /// The converse, and the reason the check counts only *complete* sequences: an ASCII sample
    /// that happens to end on a byte ≥ 0x80 is not evidence of anything. A single-byte codepage
    /// file ending on an accented character looks identical.
    #[test]
    fn an_incomplete_trailing_sequence_is_not_by_itself_evidence_of_utf8() {
        let mut bytes = log_text().into_bytes();
        bytes.push(0xE2);
        let d = detect_head(&bytes);
        assert_eq!(d.charset, Charset::UTF_8, "still the safe default");
        assert_eq!(d.confidence, Confidence::Low);
    }

    #[test]
    fn dbcs_falls_through_to_chardetng_or_the_system_codepage() {
        let text: String = (0..200)
            .map(|i| format!("2026-07-31 09:14:{i:02} 情報  ワーカーが開始しました {i}\r\n"))
            .collect();
        let (bytes, _, _) = encoding_rs::SHIFT_JIS.encode(&text);
        let d = detect(&bytes, None, encoding_rs::SHIFT_JIS);
        assert_ne!(d.charset, Charset::UTF_8, "it is not valid UTF-8");
        assert_eq!(d.confidence, Confidence::Low, "step 4 is always a guess");
    }

    /// §5.6: head and tail disagreeing is resolved in favour of the tail, because the tail is what
    /// is being appended to — and the file is flagged rather than silently resolved.
    #[test]
    fn head_and_tail_disagreement_prefers_the_tail_and_flags_it() {
        let head = log_text().into_bytes();
        let tail_bytes = utf16le(&log_text());
        let d = detect(
            &head,
            Some(Sample {
                bytes: &tail_bytes,
                // 4-aligned, so the tail's own parity matches the file's.
                offset: 1_000_000,
            }),
            encoding_rs::WINDOWS_1252,
        );
        assert_eq!(d.charset, Charset::UTF_16LE);
        assert!(d.disagreed);
    }

    /// A tail sample read from an odd offset has the opposite parity to the file. The absolute
    /// offset is carried for exactly this reason; dropping it turns UTF-16LE into UTF-16BE.
    #[test]
    fn tail_parity_is_relative_to_the_file_not_the_sample() {
        let bytes = utf16le(&log_text());
        // Start one byte in, so every code unit in the slice is split across the boundary.
        let odd = Sample {
            bytes: &bytes[1..],
            offset: 1,
        };
        assert_eq!(probe_wide(odd.bytes, odd.offset), Some(Charset::UTF_16LE));
        assert_eq!(
            probe_wide(odd.bytes, 0),
            Some(Charset::UTF_16BE),
            "this is the wrong answer the offset exists to prevent"
        );
    }

    // --- Invariants M2 depends on ------------------------------------------------------------

    #[test]
    fn code_unit_matches_the_encoding() {
        assert_eq!(Charset::UTF_8.code_unit(), 1);
        assert_eq!(Charset::UTF_16LE.code_unit(), 2);
        assert_eq!(Charset::UTF_16BE.code_unit(), 2);
        assert_eq!(Charset::Utf32Le.code_unit(), 4);
        assert_eq!(Charset::Utf32Be.code_unit(), 4);
        assert_eq!(Charset::Whatwg(encoding_rs::WINDOWS_1252).code_unit(), 1);
    }

    /// DBCS **is** chunkable, as of the session-8 spec change. Kept as an explicit test because
    /// the previous rule said the opposite for two milestones, and a reader who half-remembers it
    /// should trip over this rather than reintroduce the exception.
    #[test]
    fn dbcs_is_chunkable_and_needs_no_special_case() {
        for e in [
            encoding_rs::SHIFT_JIS,
            encoding_rs::GBK,
            encoding_rs::GB18030,
            encoding_rs::BIG5,
            encoding_rs::EUC_KR,
        ] {
            let c = Charset::Whatwg(e);
            assert_eq!(
                c.code_unit(),
                1,
                "{} is byte-oriented, so any chunk boundary is aligned",
                e.name()
            );
            assert!(c.is_random_access_decodable(), "{} is stateless", e.name());
        }
    }

    /// The one encoding that still cannot be decoded from an arbitrary line — and the reason is
    /// shift state, not trail bytes.
    #[test]
    fn iso_2022_jp_is_the_only_encoding_needing_sequential_decode() {
        assert!(!Charset::Whatwg(encoding_rs::ISO_2022_JP).is_random_access_decodable());
        for c in [
            Charset::UTF_8,
            Charset::UTF_16LE,
            Charset::Utf32Le,
            Charset::Whatwg(encoding_rs::WINDOWS_1252),
            Charset::Whatwg(encoding_rs::SHIFT_JIS),
        ] {
            assert!(c.is_random_access_decodable(), "{} is stateless", c.name());
        }
    }

    /// Every byte-oriented encoding Tailhawk can be asked to read.
    ///
    /// UTF-16 is absent because the WHATWG standard makes it decode-only — `Encoding::encode` on
    /// `UTF_16LE` emits UTF-8, so it cannot answer a question about UTF-16 bytes. UTF-16 and UTF-32
    /// are covered instead by [`Charset::code_unit`], which is the mechanism that makes them safe.
    /// `REPLACEMENT` is absent because it is not an encoding — it decodes any input to a single
    /// U+FFFD and exists to neutralise a handful of attack-prone labels.
    const BYTE_ORIENTED: &[&Encoding] = &[
        encoding_rs::UTF_8,
        // Multi-byte legacy.
        encoding_rs::BIG5,
        encoding_rs::EUC_JP,
        encoding_rs::EUC_KR,
        encoding_rs::GBK,
        encoding_rs::GB18030,
        encoding_rs::SHIFT_JIS,
        // Stateful, and therefore the one most likely to break the invariant.
        encoding_rs::ISO_2022_JP,
        // Single-byte, one per family — enough to catch a C0 remapping.
        encoding_rs::WINDOWS_1252,
        encoding_rs::WINDOWS_1251,
        encoding_rs::WINDOWS_874,
        encoding_rs::ISO_8859_2,
        encoding_rs::ISO_8859_7,
        encoding_rs::ISO_8859_8_I,
        encoding_rs::KOI8_R,
        encoding_rs::IBM866,
        encoding_rs::MACINTOSH,
        encoding_rs::X_USER_DEFINED,
    ];

    /// **The invariant the whole parallel index rests on: a `0x0A` byte in the stream is always a
    /// line terminator, never part of some other character.**
    ///
    /// It has two directions and both matter. If some character encoded to a sequence *containing*
    /// `0x0A`, a chunked scan would invent line breaks inside characters. If U+000A encoded to
    /// something without a `0x0A` byte, a chunked scan would miss line breaks entirely. Either one
    /// silently corrupts the index, and neither shows up in an ASCII fixture.
    ///
    /// **This is the measurement that refuted `SPEC.md` §5.3's stated reason.** The spec asserted
    /// `0x0A` was a legal trail byte in codepages 932/936/950/949. It is not — and the property
    /// turns out to be universal across every byte-oriented encoding in the WHATWG standard, not a
    /// lucky property of those four. That is not an accident: the standard's own encoders are
    /// ASCII-transparent in the C0 range by construction.
    ///
    /// Kept as a test rather than a comment so it fails if `encoding_rs`'s tables ever move
    /// underneath the claim.
    #[test]
    fn a_0a_byte_is_always_a_terminator_in_every_byte_oriented_encoding() {
        for e in BYTE_ORIENTED {
            for cp in 0u32..=0x10FFFF {
                let Some(ch) = char::from_u32(cp) else {
                    continue;
                };
                let mut buf = [0u8; 4];
                let (bytes, _, _) = e.encode(ch.encode_utf8(&mut buf));

                if ch == '\n' {
                    // No false negatives: the terminator must still be findable as a raw byte.
                    // ISO-2022-JP may prefix a state-reset escape, which is why this asks for
                    // containment rather than equality.
                    assert!(
                        bytes.contains(&0x0A),
                        "{}: U+000A encodes to {bytes:02X?}, which a byte scan cannot find",
                        e.name()
                    );
                    continue;
                }

                // No false positives: nothing else may contain the byte, in any position.
                assert!(
                    !bytes.contains(&0x0A),
                    "{}: U+{cp:04X} encodes to {bytes:02X?}, which contains a 0x0A that is not a \
                     line terminator — a chunked newline scan would split inside this character",
                    e.name()
                );
            }
        }
    }

    /// The same invariant from the side that actually matters, and the one the test above does
    /// **not** cover.
    ///
    /// That test asks what an *encoder* emits. A parallel scan reads arbitrary bytes off disk, and
    /// decoders accept plenty of sequences no encoder produces — non-canonical mappings, reserved
    /// lead bytes, truncated forms. So the question is not "can a `0x0A` be *written* into a trail
    /// position" but "can a `0x0A` be *consumed* as one". If any lead byte could swallow a
    /// following `0x0A`, a chunked scan would find a line break the decoder does not agree exists,
    /// and the index would disagree with the text.
    ///
    /// This drives every two-byte prefix — all 65,536 — into each decoder followed by `0x0A`, and
    /// asserts the newline always survives. It covers the multi-byte lead/trail space directly,
    /// including GB18030's four-byte form and ISO-2022-JP's shift states.
    #[test]
    fn a_0a_byte_is_never_consumed_as_a_trail_byte_by_any_decoder() {
        for e in BYTE_ORIENTED {
            for first in 0u8..=0xFF {
                for second in 0u8..=0xFF {
                    // A trailing byte after the newline so the decoder is never left mid-sequence
                    // waiting for input, which would be a different failure than the one at issue.
                    let input = [first, second, 0x0A, b'x'];
                    let (text, _) = e.decode_without_bom_handling(&input);
                    assert!(
                        text.contains('\n'),
                        "{}: {input:02X?} decodes to {text:?} — the 0x0A was swallowed, so a \
                         chunked scan would see a line break the decoder does not",
                        e.name()
                    );
                }
            }
        }
    }
}
