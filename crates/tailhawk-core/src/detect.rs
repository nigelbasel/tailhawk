//! Format detection — `SPEC.md` §6.3, E9.
//!
//! Five stages, "every stage bounded by BYTES, never by line count, so a 40 GB file opens as fast
//! as a 40 KB one." Stage 0 is `encoding.rs`. This module is stages 1–4; stage 5 (runtime
//! resilience, "no permanent lock-on") is the shell's, once records are being parsed per frame.
//!
//! ## Stage 1 — the sample window
//!
//! [`head_lines`] decodes the first [`HEAD_BYTES`] of a file into whole lines. §6.3 also wants a
//! mid and a tail sample, **asynchronously and only on local paths**, because "file heads are
//! systematically unrepresentative" — that upgrade is not here yet; the head is what is scored, and
//! `Detection::sampled` says how much was.
//!
//! ## Stage 2 — self-describing short-circuits
//!
//! `memmem`-class tests, first match wins, no scoring: `#Fields:` in the first 20 lines is **W3C
//! Extended** and its columns are taken verbatim; `{"@t":` is Serilog CLEF and `"{OriginalFormat}"`
//! is MEL Json, both of which the catalogue's JSON-lines format reads. The rest of §6.3's table —
//! XML fragment streams, wevtutil, journal export, OTLP/JSON, the Docker and CRI unwrap-and-recurse
//! cases — is recognised nowhere yet, and says so in `HANDOFF.md`.
//!
//! ## Stages 3 and 4 — the score, and the tie-break
//!
//! ```text
//! score = match_rate × (0.5 + 0.5 × field_validity) × specificity × (0.7 + 0.3 × coverage)
//! ```
//!
//! `match_rate` is first-line matches over the lines that are **not** the format's own
//! continuations — MEL Simple's message is always on the second line, and counting it as a miss
//! would keep the format that emitted it from ever scoring; that exclusion is ours, recorded in
//! `CLEANROOM.md`. `field_validity` is [`Format::validity`] over the matches. `coverage` is bytes
//! matched or continued over bytes sampled. Specificity is the format's own.
//!
//! **Acceptance requires quality ≥ 0.75 and a ≥ 15 % margin in score over the runner-up.**
//! Otherwise the answer is "not accepted", both candidates are reported, and §6.3's rule holds:
//! "silent mis-columnising is worse than no columnising."
//!
//! **Where this departs from §6.3's letter, and why.** §6.3 puts specificity *inside* the score and
//! then requires the score to reach 0.75 — under which generic timestamped text (0.20), logfmt,
//! JSON lines, Python, Serilog's console template and RFC 3164 could never be accepted, since
//! their specificity alone caps them below the threshold. Stage 4 calls specificity an
//! *ordering*, and that is the reading that works: acceptance is judged on the quality terms —
//! did it match, were the fields real, how much of the file is covered — and specificity ranks the
//! candidates and decides the margin between them, which is where a generic pattern must lose to a
//! specific one. §6.3 is amended to say so; the formula and its constants are otherwise the spec's.

use crate::encoding::Charset;
use crate::format::{catalogue, Format};
use crate::indexer::ChunkReader;
use crate::lines::LineDecoder;

/// Stage 1's head sample. 256 KiB, per §6.3.
pub const HEAD_BYTES: usize = 256 * 1024;

/// §6.3's acceptance threshold and margin.
pub const ACCEPT_SCORE: f32 = 0.75;
pub const ACCEPT_MARGIN: f32 = 1.15;

/// What a self-describing file said about itself.
#[derive(Clone, Debug, PartialEq)]
pub enum SelfDescribed {
    /// `#Fields:` — the columns, verbatim, in order.
    W3c { fields: Vec<String> },
}

/// One format's standing after scoring.
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub format: &'static Format,
    /// `quality × specificity` — what ranks candidates and sets the margin.
    pub score: f32,
    /// `match_rate × (0.5 + 0.5 × field_validity) × (0.7 + 0.3 × coverage)` — what acceptance
    /// is judged on. See the module note on why specificity is not in it.
    pub quality: f32,
    pub match_rate: f32,
    pub field_validity: f32,
    pub coverage: f32,
}

/// The outcome of detection.
#[derive(Clone, Debug)]
pub struct Detection {
    /// The format to use, **only if accepted** — otherwise the file is plain text.
    pub accepted: Option<&'static Format>,
    /// A self-describing short-circuit, which wins over scoring.
    pub self_described: Option<SelfDescribed>,
    /// Every format that matched at all, best first — for the disambiguation chip.
    pub candidates: Vec<Candidate>,
    /// Lines the score was taken over.
    pub sampled: usize,
}

impl Detection {
    /// The best-scoring candidate, accepted or not.
    pub fn best(&self) -> Option<&Candidate> {
        self.candidates.first()
    }

    /// The runner-up, when there is one.
    pub fn runner_up(&self) -> Option<&Candidate> {
        self.candidates.get(1)
    }

    /// A one-line description for a title or a chip: `Serilog (file) 92%`, or
    /// `format? Serilog (file) 61% · log4net 55%` when nothing was accepted, or nothing at all.
    pub fn describe(&self) -> Option<String> {
        if let Some(SelfDescribed::W3c { fields }) = &self.self_described {
            return Some(format!("W3C Extended ({} fields)", fields.len()));
        }
        let best = self.best()?;
        if self.accepted.is_some() {
            return Some(format!("{} {:.0}%", best.format.name, best.quality * 100.0));
        }
        let mut text = format!("format? {} {:.0}%", best.format.name, best.quality * 100.0);
        if let Some(second) = self.runner_up() {
            text.push_str(&format!(
                " · {} {:.0}%",
                second.format.name,
                second.quality * 100.0
            ));
        }
        Some(text)
    }
}

/// Stage 1: the first [`HEAD_BYTES`] of `reader`, as whole decoded lines. A trailing partial line
/// is dropped — it is not a line yet — and a leading byte-order mark is not text.
pub fn head_lines<R: ChunkReader + ?Sized>(reader: &R, charset: Charset) -> Vec<String> {
    let mut buf = vec![0u8; HEAD_BYTES];
    let mut read = 0usize;
    while read < HEAD_BYTES {
        match reader.read_at(read as u64, &mut buf[read..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => read += n,
        }
    }
    let mut lines = Vec::new();
    let mut decoder = LineDecoder::new(charset);
    decoder.push(&buf[..read], |line| {
        lines.push(line.trim_start_matches('\u{FEFF}').to_owned());
    });
    if read < HEAD_BYTES {
        decoder.finish(|line| lines.push(line.to_owned()));
    }
    lines
}

/// Stages 2–4 over a sample of lines.
pub fn detect(lines: &[String]) -> Detection {
    let self_described = short_circuit(lines);
    let total_bytes: usize = lines.iter().map(|l| l.len() + 1).sum();

    let mut candidates: Vec<Candidate> = catalogue()
        .iter()
        .filter_map(|format| score(format, lines, total_bytes))
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.format
                    .specificity
                    .partial_cmp(&a.format.specificity)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let accepted = match self_described {
        Some(SelfDescribed::W3c { .. }) => None,
        None => match (candidates.first(), candidates.get(1)) {
            (Some(best), _) if best.quality < ACCEPT_SCORE => None,
            (Some(best), Some(second)) if best.score < second.score * ACCEPT_MARGIN => None,
            (Some(best), _) => Some(best.format),
            (None, _) => None,
        },
    };

    Detection {
        accepted,
        self_described,
        candidates,
        sampled: lines.len(),
    }
}

/// Stage 2. `#Fields:` in the first 20 lines is W3C; the JSON short-circuits are handled by the
/// catalogue's JSON-lines format scoring at full marks on those files, so they need no branch here.
fn short_circuit(lines: &[String]) -> Option<SelfDescribed> {
    lines.iter().take(20).find_map(|line| {
        let rest = line.strip_prefix("#Fields:")?;
        Some(SelfDescribed::W3c {
            fields: rest.split_whitespace().map(str::to_owned).collect(),
        })
    })
}

/// Stage 3 for one format. `None` when it matched nothing at all.
fn score(format: &'static Format, lines: &[String], total_bytes: usize) -> Option<Candidate> {
    let (mut matches, mut valid, mut continuations, mut covered) = (0usize, 0usize, 0usize, 0usize);
    let mut in_record = false;
    for line in lines {
        match format.validity(line) {
            Some(ok) => {
                matches += 1;
                valid += usize::from(ok);
                covered += line.len() + 1;
                in_record = true;
            }
            None if in_record && format.is_continuation(line) => {
                continuations += 1;
                covered += line.len() + 1;
            }
            None => in_record = false,
        }
    }
    if matches == 0 {
        return None;
    }
    let considered = lines.len().saturating_sub(continuations).max(1);
    let match_rate = matches as f32 / considered as f32;
    let field_validity = valid as f32 / matches as f32;
    let coverage = if total_bytes == 0 {
        0.0
    } else {
        covered as f32 / total_bytes as f32
    };
    let quality = match_rate * (0.5 + 0.5 * field_validity) * (0.7 + 0.3 * coverage);
    let score = quality * format.specificity;
    Some(Candidate {
        format,
        score,
        quality,
        match_rate,
        field_validity,
        coverage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::by_id;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    /// A clean Serilog file is accepted as Serilog, and the generic format that also matches every
    /// line loses on specificity — which is the whole point of the table.
    #[test]
    fn a_serilog_file_is_accepted_and_generic_text_loses_on_specificity() {
        let sample = lines(
            "2026-08-16 09:14:02.117 +02:00 [INF] Started HTTP GET /api/contacts\n\
             2026-08-16 09:14:03.884 +02:00 [ERR] Failed to dispatch job 41982\n\
             System.InvalidOperationException: boom\n\
                at Api.Dispatch.Run() in Dispatch.cs:line 42\n\
             2026-08-16 09:14:04.002 +02:00 [WRN] Retry 1/3 for job 41982\n",
        );
        let d = detect(&sample);
        assert_eq!(
            d.accepted.map(|f| f.id),
            Some("serilog-file"),
            "{:?}",
            d.candidates
        );
        let best = d.best().expect("a candidate");
        assert!(
            (best.match_rate - 1.0).abs() < 1e-6,
            "continuations are excluded: {best:?}"
        );
        assert!(best.score >= ACCEPT_SCORE);
        assert!(d.describe().expect("text").starts_with("Serilog (file) "));
    }

    /// The classic false positive §6.3 names: a loose `<date> <word> <rest>` pattern matches a line
    /// whose "level" is `Starting`. `field_validity` halves that score.
    #[test]
    fn a_word_that_is_not_a_level_halves_the_generic_score() {
        let with_levels = lines("2026-08-16 09:14:02 INFO one\n2026-08-16 09:14:03 ERROR two\n");
        let without = lines("2026-08-16 09:14:02 Starting one\n2026-08-16 09:14:03 Stopping two\n");
        let a = detect(&with_levels);
        let b = detect(&without);
        let (sa, sb) = (a.best().unwrap().score, b.best().unwrap().score);
        assert!(sb < sa * 0.6, "{sb} should be well under {sa}");
    }

    /// Two formats close together is a question, not an answer: neither is accepted and both are
    /// reported.
    #[test]
    fn a_close_race_is_not_accepted_and_names_both() {
        // Half NLog, half log4net.
        let sample = lines(
            "2026-08-16 09:14:02.1170|INFO|Api.Controller|Started\n\
             2026-08-16 09:14:02,117 [12] INFO  Api.Controller - Started\n\
             2026-08-16 09:14:03.8840|ERROR|Api.Dispatch|Failed\n\
             2026-08-16 09:14:03,884 [12] ERROR Api.Dispatch - Failed\n",
        );
        let d = detect(&sample);
        assert_eq!(d.accepted, None, "{:?}", d.candidates);
        let text = d.describe().expect("text");
        assert!(text.starts_with("format? "), "{text}");
        assert!(text.contains("NLog") && text.contains("log4net"), "{text}");
    }

    /// `#Fields:` is W3C, columns verbatim, no scoring.
    #[test]
    fn a_fields_directive_is_w3c_and_wins_over_everything() {
        let sample = lines(
            "#Software: Microsoft Internet Information Services 10.0\n\
             #Fields: date time s-ip cs-method cs-uri-stem sc-status\n\
             2026-08-16 09:14:02 10.0.0.1 GET /api/contacts 200\n",
        );
        let d = detect(&sample);
        assert_eq!(
            d.self_described,
            Some(SelfDescribed::W3c {
                fields: [
                    "date",
                    "time",
                    "s-ip",
                    "cs-method",
                    "cs-uri-stem",
                    "sc-status"
                ]
                .map(str::to_owned)
                .to_vec()
            })
        );
        assert_eq!(d.accepted, None);
        assert_eq!(d.describe().as_deref(), Some("W3C Extended (6 fields)"));
    }

    /// Plain prose matches nothing and says so.
    #[test]
    fn prose_is_plain_text() {
        let d = detect(&lines("hello world\nthis is not a log\n"));
        assert!(d.candidates.is_empty());
        assert_eq!(d.accepted, None);
        assert_eq!(d.describe(), None);
    }

    /// The owner's own activity log — the dogfood file — is timestamped text with levels.
    #[test]
    fn the_agent_log_is_generic_timestamped_text() {
        let sample = lines(
            "2026-08-14T14:55:59.157Z INFO  task     next: E13 highlight rule engine\n\
             2026-08-14T17:52:51.638Z INFO  turn     resumed on your message\n\
             2026-08-17T13:03:21.018Z WARN  note     gap: sessions 17 and 18 did not write here\n",
        );
        let d = detect(&sample);
        assert_eq!(
            d.accepted.map(|f| f.id),
            Some("generic"),
            "{:?}",
            d.candidates
        );
        let f = by_id("generic").unwrap();
        let r = f.parse(&sample[2]).unwrap();
        assert_eq!(r.severity_text.as_deref(), Some("WARN"));
    }

    /// Every catalogue format's own samples, as a file, are accepted as that format — the
    /// end-to-end form of the cross-matching test.
    #[test]
    fn each_formats_samples_detect_as_that_format() {
        for f in catalogue() {
            let sample: Vec<String> = f.samples.iter().map(|(l, _)| l.to_string()).collect();
            let d = detect(&sample);
            assert_eq!(
                d.accepted.map(|a| a.id),
                Some(f.id),
                "{}: {:?}",
                f.id,
                d.candidates
                    .iter()
                    .map(|c| (c.format.id, c.score))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn the_head_sample_is_whole_lines_without_a_bom() {
        let text = "\u{FEFF}first\nsecond\npartial";
        let got = head_lines(text.as_bytes(), Charset::UTF_8);
        assert_eq!(
            got,
            ["first", "second", "partial"],
            "a short file is read whole"
        );
        let long: String = (0..20_000)
            .map(|i| format!("line {i} padding padding\n"))
            .collect();
        let got = head_lines(long.as_bytes(), Charset::UTF_8);
        assert!(got.len() < 20_000 && got.len() > 8_000);
        assert!(
            got.iter().all(|l| l.starts_with("line ")),
            "no partial line at the cut"
        );
    }
}
