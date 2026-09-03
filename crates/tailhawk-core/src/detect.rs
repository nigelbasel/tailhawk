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
//! Extended** and its columns are taken verbatim, and **a file whose lines are JSON objects sharing
//! one key order declares its columns the same way** — in its keys, through [`json_template`]. The
//! rest of §6.3's table — XML fragment streams, wevtutil, journal export, OTLP/JSON, the Docker and
//! CRI unwrap-and-recurse cases — is recognised nowhere yet, and says so in `HANDOFF.md`.
//!
//! The JSON branch adds *columns*, it does not take the format over: a record carrying nothing
//! beyond `@t`, `@l` and `@m` is what the catalogue's `ndjson` entry already reads, so it is left
//! to it. What the branch is for is the context beside the message — a Loki spill arrives with
//! `app` and `environment` on every line, and under `ndjson` alone they were fetched, written and
//! then never shown, because that format declares three columns and has nowhere to put a label.
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

/// How many keys beyond `ts`, `level` and `msg` a JSON file may turn into columns.
///
/// A presentation limit and nothing more: [`Format::parse`](crate::format::Format::parse) still
/// carries every capture into the record's attributes, and the raw line is lossless under §6.1. A
/// structured logger writing forty properties would otherwise produce a grid nobody can read.
///
/// **It is also a frame budget.** Each column is one more optional `.*?`-separated group in the
/// pattern, and `columns.rs` rebuilds the presentation per visible row per frame. Measured in
/// release on the owner's machine: over a 4 KiB line, three groups cost 96 µs and eleven cost
/// 2.45 ms — fifty rows of the latter is 120 ms, against §11's 16 ms. Six extras is the compromise;
/// the measurements are in `HANDOFF.md`.
pub const MAX_JSON_COLUMNS: usize = 6;

/// The share of sampled rows a key must appear on to be worth a column: one in this many.
///
/// A merged key order is a **union**, so it collects a single record's `key_0` alongside the `app`
/// every record carries. Without a floor those one-offs take the cap purely by being in the list,
/// and the labels that prompted all of this never reach the grid. A column blank on three rows in
/// four is not context; it is a gap with a heading. Nothing is lost by leaving it out — the key is
/// still in the record's attributes and in §6.1's raw line, which is what the detail pane reads.
pub const JSON_COLUMN_SHARE: usize = 4;

/// How many distinct top-level keys a JSON file may have before it is left to the catalogue.
///
/// [`json_template`] reconciles the key orders with a matrix of *precedes* edges, which is `n²`
/// bytes; this bounds it. It is a long way past anything that columnises usefully — the owner's
/// own estate reaches 45 distinct keys across a thousand records — so a file that trips it is a
/// file with no stable shape to find.
pub const MAX_JSON_KEYS: usize = 256;

/// What kind of value a top-level JSON key carries. Both distinctions are load-bearing.
///
/// **[`Text`](JsonValue::Text) is the only kind that can be a column.** A column is a **byte range
/// of the raw line** — that is what lets a search match be carried across into the columnised view
/// — and the only range that reads correctly for a string is the one *inside* its quotes. There is
/// no such range for `41982` that does not include the value's own punctuation, so a scalar stays
/// in the record's attributes and out of the header.
///
/// **[`Nested`](JsonValue::Nested) anywhere in the head costs the file its columns**, which is a
/// blunter rule and is deliberate. The scan below knows a nested key is not a top-level one, but
/// the *pattern* built from a template cannot: it searches the whole line, so
/// `{"ctx":{"app":"decoy"},"app":"real"}` binds the `app` column to `decoy` on every row, with
/// nothing on screen to say so. A regex cannot count braces, so the only honest answer is to
/// decline files that nest at all and leave them to the catalogue.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum JsonValue {
    /// A quoted string — the only kind a column can show.
    Text,
    /// A number, `true`, `false` or `null`.
    Scalar,
    /// An object or an array.
    Nested,
}

/// One top-level key of a JSON record, and what kind of value it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonKey {
    pub name: String,
    pub value: JsonValue,
}

impl JsonKey {
    /// Whether this key can be a column at all. See [`JsonValue`].
    pub fn is_text(&self) -> bool {
        self.value == JsonValue::Text
    }
}

/// What a self-describing file said about itself.
#[derive(Clone, Debug, PartialEq)]
pub enum SelfDescribed {
    /// `#Fields:` — the columns, verbatim, in order.
    W3c { fields: Vec<String> },
    /// A JSON record's own keys, in the order the file writes them. See [`json_template`].
    Json { keys: Vec<JsonKey> },
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
#[derive(Clone, Debug, Default)]
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
        if let Some(SelfDescribed::Json { keys }) = &self.self_described {
            return Some(format!("JSON lines ({} keys)", keys.len()));
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

/// Stages 2–4 over a sample of lines, against the catalogue alone.
pub fn detect(lines: &[String]) -> Detection {
    detect_with(lines, &[])
}

/// Stages 2–4 over a sample of lines, with `extra` formats — templates compiled from a config
/// beside the file (E11) — scored alongside the catalogue. They are candidates, not answers: a
/// stale config still has to match the file to be accepted.
pub fn detect_with(lines: &[String], extra: &[&'static Format]) -> Detection {
    let self_described = short_circuit(lines);
    let total_bytes: usize = lines.iter().map(|l| l.len() + 1).sum();

    let mut candidates: Vec<Candidate> = catalogue()
        .iter()
        .chain(extra.iter().copied())
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

    let accepted = match &self_described {
        // Self-describing wins over scoring: the file said what its columns are.
        Some(SelfDescribed::W3c { fields }) => Some(crate::format::w3c(fields)),
        Some(SelfDescribed::Json { keys }) => Some(crate::format::json_lines(keys)),
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

/// Stage 2. `#Fields:` in the first 20 lines is W3C; a file whose lines are JSON objects sharing
/// one key order describes its columns the same way, in its keys.
///
/// **A JSON file is only self-describing when it has something to describe.** A record carrying
/// nothing but `@t`, `@l` and `@m` is exactly what the catalogue's `ndjson` reader already handles,
/// so it is left to it — this branch adds columns to JSON, it does not take the format over.
fn short_circuit(lines: &[String]) -> Option<SelfDescribed> {
    if let Some(w3c) = lines.iter().take(20).find_map(|line| {
        let rest = line.strip_prefix("#Fields:")?;
        Some(SelfDescribed::W3c {
            fields: rest.split_whitespace().map(str::to_owned).collect(),
        })
    }) {
        return Some(w3c);
    }
    let keys = json_template(lines)?;
    crate::format::json_lines_adds_columns(&keys).then_some(SelfDescribed::Json { keys })
}

/// The top-level keys of one JSON object line, in the order written, with nothing from inside a
/// value. `None` when the line is not a complete object.
///
/// A hand scan rather than a parser: this runs over the head of every file opened, it needs the
/// keys and their order and nothing else, and §6.1 keeps the raw line anyway. It walks
/// `"key" : value` pairs, stepping over strings with their escapes and over nested objects and
/// arrays by depth, so a `{` inside a value is not structure and a decoy key inside a nested
/// object is not a key.
pub fn json_keys(line: &str) -> Option<Vec<JsonKey>> {
    let bytes = line.as_bytes();
    let mut at = 0usize;
    let skip_ws = |bytes: &[u8], mut at: usize| {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        at
    };
    // The end of the string starting at the opening quote `at`, one past its closing quote.
    let end_of_string = |bytes: &[u8], mut at: usize| -> Option<usize> {
        at += 1;
        while at < bytes.len() {
            match bytes[at] {
                b'\\' => at += 2,
                b'"' => return Some(at + 1),
                _ => at += 1,
            }
        }
        None
    };
    at = skip_ws(bytes, at);
    if bytes.get(at) != Some(&b'{') {
        return None;
    }
    at += 1;
    let mut keys = Vec::new();
    loop {
        at = skip_ws(bytes, at);
        match bytes.get(at) {
            Some(b'}') => return Some(keys),
            Some(b'"') => {}
            _ => return None,
        }
        let key_end = end_of_string(bytes, at)?;
        let name = line.get(at + 1..key_end - 1)?.to_owned();
        at = skip_ws(bytes, key_end);
        if bytes.get(at) != Some(&b':') {
            return None;
        }
        at = skip_ws(bytes, at + 1);
        let value = match bytes.get(at) {
            Some(b'"') => JsonValue::Text,
            Some(b'{') | Some(b'[') => JsonValue::Nested,
            _ => JsonValue::Scalar,
        };
        at = match bytes.get(at) {
            Some(b'"') => end_of_string(bytes, at)?,
            Some(b'{') | Some(b'[') => {
                let mut depth = 0usize;
                loop {
                    match bytes.get(at)? {
                        b'"' => at = end_of_string(bytes, at)?,
                        b'{' | b'[' => {
                            depth += 1;
                            at += 1;
                        }
                        b'}' | b']' => {
                            depth -= 1;
                            at += 1;
                            if depth == 0 {
                                break at;
                            }
                        }
                        _ => at += 1,
                    }
                }
            }
            Some(_) => {
                while at < bytes.len() && !matches!(bytes[at], b',' | b'}') {
                    at += 1;
                }
                at
            }
            None => return None,
        };
        keys.push(JsonKey { name, value });
        at = skip_ws(bytes, at);
        match bytes.get(at) {
            Some(b',') => at += 1,
            Some(b'}') => return Some(keys),
            _ => return None,
        }
    }
}

/// The key order a whole file can be read by, or `None` when it has no single one.
///
/// **The order is merged across the sampled lines — see [`merged_order`], which is where the
/// reasoning lives and where the first construction of this went wrong.**
///
/// What the merge is guarding against is quieter than it looks. Every group in the pattern is
/// optional and the whole thing is wrapped in `^\s*\{ … \}\s*$`, so a line whose keys run the other
/// way still *matches* — it does not become a §6.4 continuation. What it does is bind the wrong
/// groups: a line writing `@m` before `app` gives up its message cell, because the message group
/// sits after the `app` group and the text it wanted has already been consumed. **Empty cells on
/// some rows and not others**, with nothing on screen to say the file was read wrongly.
///
/// **The sample is the head alone** — [`HEAD_BYTES`], through [`head_lines`] — so this is evidence
/// about a file, not a proof about it. A file that changes its key order a gigabyte in loses cells
/// from that point and says nothing. Re-reading the format mid-file is §6.3's stage 5, which
/// `HANDOFF.md` lists as not built.
pub fn json_template(lines: &[String]) -> Option<Vec<JsonKey>> {
    // **Every non-blank sampled line must be an object, and a line that is not costs the file its
    // columns.** Collecting only the lines that happen to be JSON would let a *trace* of JSON take
    // the file over: stage 2 wins unconditionally over scoring, so one CLEF line in the head of a
    // Serilog log — or one pretty-printed payload dumped on its own line, which is a shape every
    // estate has — would make that whole file one JSON row with every text line a §6.4
    // continuation of it.
    let mut sampled: Vec<Vec<JsonKey>> = Vec::new();
    for line in lines.iter().filter(|l| !l.trim().is_empty()) {
        sampled.push(json_keys(line)?);
    }
    // Nesting anywhere in the head costs the file its columns — see [`JsonValue::Nested`], which
    // is where the reason lives.
    if sampled
        .iter()
        .flatten()
        .any(|key| key.value == JsonValue::Nested)
    {
        return None;
    }
    let order = merged_order(&sampled)?;
    if order.is_empty() {
        return None;
    }
    // **Extras are ranked by how many rows carry them**, because a merged order is a union and a
    // union collects the rare along with the universal: one record's `key_0` sits in the same list
    // as the `app` every record has, and a column present on two rows in four hundred is a column
    // of blanks. Ties keep merged order, so a file whose keys are all universal — which a spill's
    // are — is presented in the order it writes them.
    let rows = |name: &str| {
        sampled
            .iter()
            .filter(|keys| keys.iter().any(|key| key.name == name))
            .count()
    };
    let floor = sampled.len().div_ceil(JSON_COLUMN_SHARE);
    let mut extras: Vec<(usize, usize)> = order
        .iter()
        .enumerate()
        .filter(|(_, key)| key.is_text() && crate::format::understood_key(&key.name).is_none())
        .map(|(at, key)| (at, rows(&key.name)))
        .filter(|(_, rows)| *rows >= floor)
        .collect();
    extras.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    extras.truncate(MAX_JSON_COLUMNS);
    let kept: Vec<usize> = extras.into_iter().map(|(at, _)| at).collect();
    Some(
        order
            .into_iter()
            .enumerate()
            .filter(|(at, key)| {
                key.is_text()
                    && (crate::format::understood_key(&key.name).is_some() || kept.contains(at))
            })
            .map(|(_, key)| key)
            .collect(),
    )
}

/// The one key order every sampled line agrees with, or `None` when they cannot be reconciled.
///
/// **A union, and not "the line that has them all".** That was the first construction and it
/// cannot survive real data: Loki merges each record's own structured metadata into its stream
/// object — in this estate, the Serilog message properties, so `ElapsedMs` and `IntegrationId` and
/// `Topic` appear on the records that have them and nowhere else. **No line is ever a superset of
/// the rest.** Requiring one meant a live spill silently kept the catalogue's three columns: the
/// labels were fetched, written to disk, and still not shown.
///
/// **A topological sort, and not a greedy insert**, which was the second construction and failed
/// on the same data for a subtler reason. Walking each line against a cursor and inserting an
/// unseen key where the cursor stands works only while consecutive lines share keys: two records
/// with disjoint property sets give the second one no anchor, so its keys land wherever the cursor
/// happens to be — ahead of keys that should precede them. A third record carrying both then looks
/// like a reordering and the file is refused. Sorting the labels in `clef_line` does not save it,
/// because the fault is in the merge and not in the input.
///
/// So the constraints are collected instead of applied: each line's consecutive keys give a
/// *precedes* edge, and the order is any topological order of the whole graph. **A cycle is the
/// genuine disagreement** — two lines writing the same pair of keys in opposite orders — and is
/// refused. Ties are broken by name, so a file whose lines are each written in sorted order, which
/// a spill's are, comes out in that order rather than in an arbitrary consistent one.
fn merged_order(sampled: &[Vec<JsonKey>]) -> Option<Vec<JsonKey>> {
    let mut nodes: Vec<JsonKey> = Vec::new();
    for key in sampled.iter().flatten() {
        if !nodes.iter().any(|seen| seen.name == key.name) {
            if nodes.len() == MAX_JSON_KEYS {
                return None;
            }
            nodes.push(key.clone());
        }
    }
    let n = nodes.len();
    let at = |name: &str| nodes.iter().position(|seen| seen.name == name);
    let mut precedes = vec![false; n * n];
    for keys in sampled {
        for pair in keys.windows(2) {
            let (before, after) = (at(&pair[0].name)?, at(&pair[1].name)?);
            precedes[before * n + after] = true;
        }
    }
    let mut waiting: Vec<usize> = (0..n)
        .map(|after| (0..n).filter(|before| precedes[before * n + after]).count())
        .collect();
    let mut placed = vec![false; n];
    let mut order = Vec::with_capacity(n);
    for _ in 0..n {
        // Ties by name, so a file whose lines are each written in sorted order — which a spill's
        // are, because `lokiwire::clef_line` sorts its labels — comes out in that same order rather
        // than in an arbitrary one that happens to satisfy the constraints.
        let next = (0..n)
            .filter(|i| !placed[*i] && waiting[*i] == 0)
            .min_by(|a, b| nodes[*a].name.cmp(&nodes[*b].name))?;
        placed[next] = true;
        for after in 0..n {
            if precedes[next * n + after] {
                waiting[after] -= 1;
            }
        }
        order.push(nodes[next].clone());
    }
    // A topological order already guarantees this — each line's chain forces its own keys' relative
    // positions — so it is a check on this function rather than on the file. It is the property the
    // pattern actually depends on, and it costs one pass.
    let subsequence = |keys: &[JsonKey]| {
        let mut cursor = 0usize;
        keys.iter().all(|key| {
            match order[cursor..]
                .iter()
                .position(|seen| seen.name == key.name)
            {
                Some(found) => {
                    cursor += found + 1;
                    true
                }
                None => false,
            }
        })
    };
    sampled
        .iter()
        .all(|keys| subsequence(keys))
        .then_some(order)
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
    use crate::record::SeverityBand;

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
        let w3c = d.accepted.expect("a format built from the directive");
        assert_eq!(w3c.id, "w3c");
        let r = w3c
            .parse("2026-08-16 09:14:02 10.0.0.1 GET /api/contacts 200")
            .expect("row");
        assert_eq!(
            r.severity_number.map(|s| s.band()),
            Some(SeverityBand::Info)
        );
        assert_eq!(w3c.titles.map(|t| t[2]), Some("s-ip"));
        assert!(
            w3c.is_continuation("#Software: x"),
            "directives are not records"
        );
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

    fn names(keys: &[JsonKey]) -> Vec<&str> {
        keys.iter().map(|k| k.name.as_str()).collect()
    }

    /// The top-level keys, in the order written, and nothing from inside a value.
    #[test]
    fn the_keys_of_an_object_are_read_in_order_and_nesting_is_skipped() {
        let keys = json_keys(
            r#"{"@t":"2026-09-03T07:00:00Z","@m":"hello","inner":{"@t":"decoy","app":"decoy"},"tags":["a","b"],"n":41982,"app":"identity"}"#,
        )
        .expect("an object");
        assert_eq!(names(&keys), ["@t", "@m", "inner", "tags", "n", "app"]);
        let string_valued: Vec<&str> = keys
            .iter()
            .filter(|k| k.is_text())
            .map(|k| k.name.as_str())
            .collect();
        assert_eq!(string_valued, ["@t", "@m", "app"]);
    }

    /// **One JSON line in a text file must not take the file over.** Stage 2 wins unconditionally
    /// over scoring — it has no coverage floor the way stage 3 does — so collecting only the lines
    /// that happen to be JSON let a *trace* of JSON decide the format for all of them: a Serilog
    /// log with one CLEF line in its head opened as **a single row**, every text line a §6.4
    /// continuation of it. The "payload follows" shape, where an ordinary log dumps a JSON body on
    /// its own line, is the same bug and is far more common.
    #[test]
    fn one_json_line_among_text_lines_does_not_take_the_file_over() {
        let mut text: Vec<String> = (0..30)
            .map(|i| format!("2026-08-16 09:14:{i:02} INFO  BYM.Api  request served"))
            .collect();
        text.insert(
            7,
            "{\"@t\":\"2026-09-03T07:00:00Z\",\"app\":\"identity\",\"@m\":\"one\"}".to_owned(),
        );
        assert_eq!(json_template(&text), None);

        let d = detect(&text);
        assert_eq!(
            d.accepted.map(|f| f.id),
            Some("generic"),
            "the text lines must keep their own format: {:?}",
            d.candidates
        );
    }

    /// A blank line is not evidence against a JSON file — files end with one.
    #[test]
    fn a_blank_line_does_not_cost_a_json_file_its_columns() {
        let sample = lines(
            "{\"@t\":\"a\",\"app\":\"identity\",\"@m\":\"one\"}\n\
             \n\
             {\"@t\":\"b\",\"app\":\"identity\",\"@m\":\"two\"}\n",
        );
        assert!(json_template(&sample).is_some());
    }

    /// **A nested object steals the column from the top-level key of the same name.** The scan
    /// knows `app` inside `ctx` is not a top-level key; the *pattern* built from the template
    /// cannot, because it searches the whole line and binds to the first `"app":"` it finds. Every
    /// row would show the decoy with nothing to say so, and a regex cannot count braces — so a
    /// file that nests at all is declined and left to the catalogue.
    #[test]
    fn a_file_that_nests_is_declined_rather_than_reading_the_inner_key() {
        let sample = lines(
            "{\"@t\":\"a\",\"ctx\":{\"app\":\"DECOY\"},\"app\":\"real\",\"@m\":\"one\"}\n\
             {\"@t\":\"b\",\"ctx\":{\"app\":\"DECOY\"},\"app\":\"real\",\"@m\":\"two\"}\n",
        );
        assert_eq!(json_template(&sample), None);
        assert_ne!(detect(&sample).accepted.map(|f| f.id), Some("json-lines"));
    }

    /// A key whose name contains an escaped quote must not end the name early, and a value
    /// containing a brace must not be read as structure.
    #[test]
    fn an_escaped_quote_in_a_key_and_a_brace_in_a_value_are_both_survived() {
        let keys = json_keys(r#"{"a\"b":"}{","c":"d"}"#).expect("an object");
        assert_eq!(names(&keys), [r#"a\"b"#, "c"]);
    }

    /// Not an object, and a truncated one, are both "no keys" rather than a wrong answer.
    #[test]
    fn a_non_object_and_a_truncated_object_yield_no_keys() {
        assert_eq!(json_keys("2026-08-16 09:14:02 INFO one"), None);
        assert_eq!(json_keys(r#"["a","b"]"#), None);
        assert_eq!(json_keys(r#"{"a":"b","c"#), None);
        assert_eq!(json_keys("{}"), Some(Vec::new()));
    }

    /// **The shape a real Loki spill actually has, and the one the first construction could not
    /// read.** Every record merges its own structured metadata into its stream object, so one line
    /// carries `key_0` and its neighbours do not — no line is a superset of the rest. Requiring one
    /// meant a live spill silently kept the catalogue's three columns: the labels were fetched,
    /// written to disk, and still not shown. These two lines are the first two of a real pull,
    /// with the values replaced.
    #[test]
    fn a_key_only_one_record_carries_does_not_cost_the_file_its_columns() {
        // **Three lines, and the third is what makes this a real fixture.** With only the first
        // two, line two happens to hold every key line one has — so the superset rule this replaced
        // would pass, and a mutation back to it went green. It takes a *second* record with a
        // one-off key of its own before no line contains them all, which is the case a 560-line
        // sample is full of and a two-line fixture cannot reach.
        let sample = lines(
            "{\"@t\":\"2026-09-03T08:29:55.7228166Z\",\"@l\":\"info\",\"app\":\"nurtur-contacts-job-manager\",\"environment\":\"live\",\"message_template_text\":\"Updating Worker Threads\",\"observed_timestamp\":\"1788424195722816600\",\"scope_name\":\"QueueManagerWorker\",\"severity_number\":\"9\",\"@m\":\"Updating Worker Threads\"}\n\
             {\"@t\":\"2026-09-03T08:29:55.7228497Z\",\"@l\":\"info\",\"app\":\"nurtur-contacts-job-manager\",\"environment\":\"live\",\"key_0\":\"0\",\"message_template_text\":\"Removing {0} Queues\",\"observed_timestamp\":\"1788424195722849700\",\"scope_name\":\"QueueManagerWorker\",\"severity_number\":\"9\",\"@m\":\"Removing 0 Queues\"}\n\
             {\"@t\":\"2026-09-03T08:29:55.7229120Z\",\"@l\":\"warn\",\"app\":\"nurtur-identity-server\",\"environment\":\"live\",\"key_queue\":\"contacts\",\"message_template_text\":\"Queue {queue} is deep\",\"observed_timestamp\":\"1788424195722912000\",\"scope_name\":\"TokenEndpoint\",\"severity_number\":\"13\",\"@m\":\"Queue contacts is deep\"}\n",
        );
        let template = json_template(&sample).expect("a real spill must yield a template");
        assert!(names(&template).contains(&"app"), "{:?}", names(&template));
        assert!(
            names(&template).contains(&"environment"),
            "{:?}",
            names(&template)
        );

        let format = detect(&sample).accepted.expect("a format");
        assert_eq!(format.id, "json-lines");
        let titles: Vec<&str> = format.column_titles().collect();
        let at = |title: &str| titles.iter().position(|t| *t == title).expect(title);
        let read = |line: &String, title: &str| {
            format.fields(line).expect("a first line")[at(title)]
                .clone()
                .map(|r| line[r].to_owned())
        };
        assert_eq!(
            read(&sample[1], "app").as_deref(),
            Some("nurtur-contacts-job-manager")
        );
        assert_eq!(read(&sample[1], "environment").as_deref(), Some("live"));
        assert_eq!(read(&sample[1], "@m").as_deref(), Some("Removing 0 Queues"));
        assert_eq!(titles.last(), Some(&"@m"), "{titles:?}");
    }

    /// **Records with disjoint property sets, which is what a real estate sends.** Loki carries
    /// each record's Serilog message properties as structured metadata, so two consecutive records
    /// routinely share no key beyond the fixed ones. A merge that inserts an unseen key wherever
    /// its cursor happens to stand has no anchor for the second record, drops its keys ahead of
    /// ones that should precede them, and then refuses the file when a third record carries both in
    /// their real order. Collecting the constraints and sorting them topologically has no such
    /// dependence on which line came first.
    ///
    /// This is the shape that defeated the second construction on live data — `Topic` on line 269
    /// of a real pull, against an order that had already misplaced it.
    #[test]
    fn records_that_share_no_properties_still_reconcile_to_one_order() {
        let sample = lines(
            "{\"@t\":\"a\",\"@l\":\"info\",\"ElapsedMs\":\"12\",\"@m\":\"one\"}\n\
             {\"@t\":\"b\",\"@l\":\"info\",\"Topic\":\"contacts\",\"@m\":\"two\"}\n\
             {\"@t\":\"c\",\"@l\":\"warn\",\"ElapsedMs\":\"31\",\"Topic\":\"leads\",\"@m\":\"three\"}\n",
        );
        let template = json_template(&sample).expect("a template");
        let got = names(&template);
        assert!(
            got.contains(&"ElapsedMs") && got.contains(&"Topic"),
            "{got:?}"
        );
        assert!(
            got.iter().position(|k| *k == "ElapsedMs") < got.iter().position(|k| *k == "Topic"),
            "line three writes ElapsedMs before Topic: {got:?}"
        );

        let format = detect(&sample).accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        let at = |t: &str| titles.iter().position(|x| *x == t).expect(t);
        let ranges = format.fields(&sample[2]).expect("a first line");
        assert_eq!(
            ranges[at("Topic")].clone().map(|r| &sample[2][r]),
            Some("leads")
        );
        assert_eq!(
            ranges[at("ElapsedMs")].clone().map(|r| &sample[2][r]),
            Some("31")
        );
    }

    /// Two lines writing the same pair of keys in opposite orders is a cycle in the constraints,
    /// and there is no order that serves both. Refused, rather than one of them read wrongly.
    #[test]
    fn a_genuine_disagreement_about_order_is_a_cycle_and_is_refused() {
        let sample = lines(
            "{\"@t\":\"a\",\"app\":\"identity\",\"region\":\"uksouth\",\"@m\":\"one\"}\n\
             {\"@t\":\"b\",\"region\":\"uksouth\",\"app\":\"identity\",\"@m\":\"two\"}\n",
        );
        assert_eq!(json_template(&sample), None);
    }

    /// **The order is a merge, and this is the property that says so.** Taking any single line —
    /// the first, or the longest — cannot see a key that line does not carry. Here the longest
    /// record is the one with two one-off keys, and `region` is on three rows out of four but not
    /// on that one: read from the longest line it would never be a column at all, and the merge is
    /// what puts it back.
    #[test]
    fn a_key_the_longest_line_lacks_still_becomes_a_column() {
        let sample = lines(
            "{\"@t\":\"a\",\"app\":\"identity\",\"one_off_x\":\"1\",\"one_off_y\":\"2\",\"@m\":\"one\"}\n\
             {\"@t\":\"b\",\"app\":\"identity\",\"region\":\"uksouth\",\"@m\":\"two\"}\n\
             {\"@t\":\"c\",\"app\":\"identity\",\"region\":\"uksouth\",\"@m\":\"three\"}\n\
             {\"@t\":\"d\",\"app\":\"identity\",\"region\":\"ukwest\",\"@m\":\"four\"}\n",
        );
        let template = json_template(&sample).expect("a template");
        assert!(
            names(&template).contains(&"region"),
            "the longest line has no region: {:?}",
            names(&template)
        );

        let format = detect(&sample).accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        let at = titles.iter().position(|t| *t == "region").expect("region");
        let ranges = format.fields(&sample[3]).expect("a first line");
        assert_eq!(
            ranges[at].clone().map(|r| &sample[3][r]),
            Some("ukwest"),
            "{titles:?}"
        );
    }

    /// **Rare keys lose their column to common ones.** A merged order is a union, so it collects
    /// the one record's `key_0` alongside the `app` every record has; without ranking, a handful of
    /// one-off keys would take the cap and leave the labels out of the grid.
    #[test]
    fn a_key_almost_no_row_carries_loses_its_column_to_one_they_all_do() {
        let mut sample: Vec<String> = (0..20)
            .map(|i| {
                format!(
                    "{{\"@t\":\"a{i}\",\"app\":\"identity\",\"environment\":\"live\",\"@m\":\"m{i}\"}}"
                )
            })
            .collect();
        sample[3] = "{\"@t\":\"a3\",\"app\":\"identity\",\"environment\":\"live\",\"one_off_a\":\"x\",\"one_off_b\":\"x\",\"one_off_c\":\"x\",\"one_off_d\":\"x\",\"one_off_e\":\"x\",\"one_off_f\":\"x\",\"@m\":\"m3\"}".to_owned();
        let template = json_template(&sample).expect("a template");
        assert!(names(&template).contains(&"app"), "{:?}", names(&template));
        assert!(
            names(&template).contains(&"environment"),
            "{:?}",
            names(&template)
        );
        assert!(
            !names(&template).contains(&"one_off_a"),
            "{:?}",
            names(&template)
        );
    }

    /// A key most lines lack still merges into the order, and it lands where the line that has it
    /// says it belongs. Loki writes `@l` only for a stream that carries a level, so the first line
    /// of a spill is routinely short a key most of the rest have.
    #[test]
    fn a_key_missing_from_the_first_line_still_merges_into_the_order() {
        let sample = lines(
            "{\"@t\":\"2026-09-03T07:00:00Z\",\"@m\":\"one\",\"app\":\"identity\"}\n\
             {\"@t\":\"2026-09-03T07:00:01Z\",\"@l\":\"Error\",\"@m\":\"two\",\"app\":\"identity\",\"environment\":\"live\"}\n\
             {\"@t\":\"2026-09-03T07:00:02Z\",\"@m\":\"three\",\"app\":\"identity\"}\n",
        );
        assert_eq!(
            names(&json_template(&sample).expect("a template")),
            ["@t", "@l", "@m", "app", "environment"]
        );
    }

    /// **The guard that stops rows silently merging.** The pattern built from a template is
    /// ordered, so a line whose keys run the other way does not match the first-line anchor — and
    /// §6.4 then makes it a *continuation* of the row above rather than a row of its own. Lines
    /// disagreeing on order is exactly the signal that no ordered pattern can serve the file, so
    /// there is no template and the plain reader keeps the file.
    #[test]
    fn keys_in_a_different_order_are_refused_rather_than_merging_rows() {
        let sample = lines(
            "{\"@t\":\"2026-09-03T07:00:00Z\",\"app\":\"identity\",\"environment\":\"live\"}\n\
             {\"@t\":\"2026-09-03T07:00:01Z\",\"environment\":\"live\",\"app\":\"identity\"}\n",
        );
        assert_eq!(json_template(&sample), None);
    }

    /// A key some lines lack is fine — that is a subsequence, and the column is simply empty on
    /// those rows. Only *reordering* is fatal.
    #[test]
    fn a_key_missing_from_some_lines_still_yields_a_template() {
        let sample = lines(
            "{\"@t\":\"a\",\"@l\":\"Error\",\"@m\":\"one\",\"app\":\"identity\"}\n\
             {\"@t\":\"b\",\"@m\":\"two\"}\n",
        );
        assert_eq!(
            names(&json_template(&sample).expect("a template")),
            ["@t", "@l", "@m", "app"]
        );
    }

    /// A file of plain text has no template at all, so nothing here can disturb it.
    #[test]
    fn a_file_that_is_not_json_has_no_template() {
        assert_eq!(
            json_template(&lines("2026-08-16 09:14:02 INFO one\n")),
            None
        );
    }

    /// **What the whole change is for.** A Loki spill arrives with `app` and `environment` on
    /// every line, and before this they were fetched, written and then never shown: the `ndjson`
    /// format declares `ts`, `level` and `msg` and has nowhere to put a label. The file describes
    /// its own columns, so it is read the way a W3C file with a `#Fields:` line is.
    #[test]
    fn a_loki_spill_is_columnised_by_its_own_labels() {
        let sample = lines(
            "{\"@t\":\"2026-09-03T07:00:00.1Z\",\"@l\":\"Information\",\"app\":\"nurtur-identity-server\",\"environment\":\"live\",\"@m\":\"token issued\"}\n\
             {\"@t\":\"2026-09-03T07:00:01.2Z\",\"@l\":\"Error\",\"app\":\"campaign-editor-api\",\"environment\":\"live\",\"@m\":\"boom\"}\n",
        );
        let d = detect(&sample);
        let format = d.accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        assert_eq!(titles, ["@t", "@l", "app", "environment", "@m"]);

        let ranges = format.fields(&sample[0]).expect("a first line");
        let read = |i: usize| ranges[i].clone().map(|r| &sample[0][r]);
        assert_eq!(read(2), Some("nurtur-identity-server"));
        assert_eq!(read(3), Some("live"));
        assert_eq!(read(4), Some("token issued"));
    }

    /// The two apps are told apart by a column, which is the request this was built for: with only
    /// `ts`, `level` and `msg` on screen, one window holding two services is unreadable.
    #[test]
    fn two_apps_in_one_spill_read_differently_in_the_app_column() {
        let sample = lines(
            "{\"@t\":\"2026-09-03T07:00:00.1Z\",\"app\":\"nurtur-identity-server\",\"@m\":\"one\"}\n\
             {\"@t\":\"2026-09-03T07:00:01.2Z\",\"app\":\"campaign-editor-api\",\"@m\":\"two\"}\n",
        );
        let format = detect(&sample).accepted.expect("a format");
        let app_of = |line: &String| {
            let r = format.fields(line).expect("a first line")[1].clone();
            r.map(|r| line[r].to_owned())
        };
        assert_eq!(
            app_of(&sample[0]).as_deref(),
            Some("nurtur-identity-server")
        );
        assert_eq!(app_of(&sample[1]).as_deref(), Some("campaign-editor-api"));
    }

    /// A key whose value is not a string does not become a column. A column is a **byte range of
    /// the raw line**, so the only honest range for `41982` or `{"a":1}` would include the value's
    /// own punctuation; a number would read `41982` but an object would read its braces and a
    /// string would need its quotes stripped, and one of those three has to be wrong. Strings are
    /// what labels are.
    #[test]
    fn a_key_that_is_not_a_string_is_not_offered_as_a_column() {
        let sample = lines(
            "{\"@t\":\"a\",\"job\":41982,\"app\":\"identity\",\"@m\":\"one\"}\n\
             {\"@t\":\"b\",\"job\":41983,\"app\":\"identity\",\"@m\":\"two\"}\n",
        );
        let format = detect(&sample).accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        assert!(!titles.contains(&"job"), "{titles:?}");
        assert_eq!(titles, ["@t", "app", "@m"]);
    }

    /// A wide record does not become a wide grid. The cap is a presentation limit, not a parsing
    /// one — `parse` still carries every capture into the record's attributes.
    #[test]
    fn a_record_with_many_keys_is_capped_to_a_readable_number_of_columns() {
        let mut line = String::from("{\"@t\":\"a\",\"@m\":\"one\"");
        for i in 0..40 {
            line.push_str(&format!(",\"k{i}\":\"v{i}\""));
        }
        line.push('}');
        let sample = lines(&format!("{line}\n{line}\n"));
        let format = detect(&sample).accepted.expect("a format");
        assert!(
            format.columns.len() <= MAX_JSON_COLUMNS + 3,
            "{:?}",
            format.column_titles().collect::<Vec<_>>()
        );
    }

    /// **The message is the last column whatever the file does with it**, because §2.5 gives the
    /// last column the free remainder of the width. Bunyan writes `name, hostname, pid, level,
    /// msg, time, v` — left in file order the message was capped at `columns::MAX_CELLS` and a
    /// twenty-character timestamp was handed the rest of the window.
    #[test]
    fn the_message_column_is_last_even_when_the_file_writes_it_in_the_middle() {
        let sample = lines(
            "{\"name\":\"api\",\"hostname\":\"box\",\"pid\":41,\"level\":\"error\",\"msg\":\"one\",\"time\":\"2026-09-03T07:00:00Z\"}\n\
             {\"name\":\"api\",\"hostname\":\"box\",\"pid\":42,\"level\":\"error\",\"msg\":\"two\",\"time\":\"2026-09-03T07:00:01Z\"}\n",
        );
        let format = detect(&sample).accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        assert_eq!(
            titles.last(),
            Some(&"msg"),
            "the message must take the remainder: {titles:?}"
        );
        let ranges = format.fields(&sample[0]).expect("a first line");
        let last = ranges.last().expect("a last column").clone();
        assert_eq!(last.map(|r| &sample[0][r]), Some("one"));
    }

    /// **A key that sanitises onto a role's name must not take the role.** `msg ` — with the
    /// trailing space a real logger will eventually emit — sanitises to `msg`, and binding it
    /// first gave the message column to the decoy and dropped `@m` from the grid altogether:
    /// message gone, and `app` inheriting the free-remainder width behind it.
    #[test]
    fn a_key_that_sanitises_onto_a_role_does_not_steal_it() {
        let sample = lines(
            "{\"@t\":\"a\",\"msg \":\"NOT THE MESSAGE\",\"app\":\"identity\",\"@m\":\"the real message\"}\n\
             {\"@t\":\"b\",\"msg \":\"NOT THE MESSAGE\",\"app\":\"identity\",\"@m\":\"also real\"}\n",
        );
        let format = detect(&sample).accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        assert!(titles.contains(&"@m"), "the message survives: {titles:?}");
        assert!(
            titles.contains(&"msg "),
            "and so does the decoy: {titles:?}"
        );

        let ranges = format.fields(&sample[0]).expect("a first line");
        let at = titles.iter().position(|t| *t == "@m").expect("@m");
        assert_eq!(
            ranges[at].clone().map(|r| &sample[0][r]),
            Some("the real message")
        );
    }

    /// **Two keys that sanitise to the same capture name are both kept, suffixed.** Dropping the
    /// second loses a column of real data with nothing on screen to say so — and any two non-ASCII
    /// keys sanitise to nothing at all, so this is not an exotic case in an estate with
    /// non-English field names.
    #[test]
    fn keys_that_sanitise_alike_are_disambiguated_rather_than_dropped() {
        let sample = lines(
            "{\"@t\":\"a\",\"a.b\":\"first\",\"a-b\":\"second\",\"@m\":\"one\"}\n\
             {\"@t\":\"b\",\"a.b\":\"first\",\"a-b\":\"second\",\"@m\":\"two\"}\n",
        );
        let format = detect(&sample).accepted.expect("a format");
        let titles: Vec<&str> = format.column_titles().collect();
        assert!(
            titles.contains(&"a.b") && titles.contains(&"a-b"),
            "{titles:?}"
        );

        let ranges = format.fields(&sample[0]).expect("a first line");
        let read = |title: &str| {
            let at = titles.iter().position(|t| *t == title).expect(title);
            ranges[at].clone().map(|r| sample[0][r].to_owned())
        };
        assert_eq!(read("a.b").as_deref(), Some("first"));
        assert_eq!(read("a-b").as_deref(), Some("second"));
    }

    /// A JSON file with no message key at all is left to the catalogue: with the message column
    /// absent, §2.5's free remainder would fall to whichever label happened to be last.
    #[test]
    fn a_json_file_with_no_message_key_is_left_to_the_catalogue() {
        let sample = lines(
            "{\"@t\":\"2026-09-03T07:00:00Z\",\"app\":\"identity\",\"environment\":\"live\"}\n\
             {\"@t\":\"2026-09-03T07:00:01Z\",\"app\":\"identity\",\"environment\":\"live\"}\n",
        );
        assert_ne!(detect(&sample).accepted.map(|f| f.id), Some("json-lines"));
    }

    /// A JSON file with nothing but the three understood keys gains no columns, so the catalogue's
    /// own `ndjson` reader is what a plain CLEF file still gets — this adds columns, it does not
    /// replace the format.
    #[test]
    fn a_json_file_with_no_extra_keys_is_left_to_the_catalogue() {
        let sample = lines(
            "{\"@t\":\"2026-09-03T07:00:00Z\",\"@l\":\"Error\",\"@m\":\"one\"}\n\
             {\"@t\":\"2026-09-03T07:00:01Z\",\"@l\":\"Error\",\"@m\":\"two\"}\n",
        );
        assert_eq!(detect(&sample).accepted.map(|f| f.id), Some("ndjson"));
    }
}

#[cfg(test)]
mod corpus {
    //! Detection over a real file named by `TAILHAWK_DETECT_FILE` — the dogfood corpus, whose lines
    //! stay out of the repo. Prints the verdict; asserts nothing but that it ran.
    //!
    //! ```text
    //! TAILHAWK_DETECT_FILE=C:\path\to\app.log cargo test -p tailhawk-core --lib detect::corpus -- --ignored --nocapture
    //! ```
    use super::*;

    #[test]
    #[ignore = "needs TAILHAWK_DETECT_FILE"]
    fn what_the_detector_makes_of_a_real_file() {
        let Some(path) = std::env::var_os("TAILHAWK_DETECT_FILE") else {
            eprintln!("skipped: set TAILHAWK_DETECT_FILE");
            return;
        };
        let bytes = std::fs::read(&path).expect("read the file");
        let charset = crate::encoding::detect(&bytes, None, encoding_rs::WINDOWS_1252).charset;
        let lines = head_lines(&bytes[..], charset);
        let d = detect(&lines);
        println!(
            "{}: {} lines sampled, accepted={:?}, {}",
            path.to_string_lossy(),
            d.sampled,
            d.accepted.map(|f| f.id),
            d.describe().unwrap_or_else(|| "plain text".into())
        );
        if let Some(format) = d.accepted {
            println!(
                "  columns: {:?}",
                format.column_titles().collect::<Vec<_>>()
            );
        }
        // **Why the JSON branch declined, when it did.** Stage 2 is silent by design — it either
        // describes the file or leaves it to the catalogue — and a real file that *should* have
        // been columnised and was not leaves a reader nothing to go on. Twice the answer has been
        // in the data rather than in the code, and both times it took reading the file to find.
        //
        // **It asks the real functions and re-implements none of them.** An earlier draft of this
        // walked its own copy of the merge, and when the merge changed the diagnostic went on
        // reporting the old algorithm's complaint about a file the new one reads perfectly — a
        // harness disagreeing with the product, which is a failure this project has had three
        // times and does not need a fourth.
        match json_template(&lines) {
            Some(template) => println!(
                "  json: a template of {} keys, {:?}",
                template.len(),
                template.iter().map(|k| k.name.as_str()).collect::<Vec<_>>()
            ),
            None => {
                let culprit = lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| !l.trim().is_empty())
                    .find_map(|(n, l)| match json_keys(l) {
                        None => Some(format!("line {n} is not an object")),
                        Some(keys) => keys
                            .iter()
                            .find(|k| k.value == JsonValue::Nested)
                            .map(|k| format!("line {n} nests at {:?}", k.name)),
                    });
                match culprit {
                    Some(why) => println!("  json: no template — {why}"),
                    None => println!(
                        "  json: no template — the sampled lines cannot be reconciled to one key \
                         order, or carry nothing beyond a timestamp, level and message"
                    ),
                }
            }
        }
        for c in &d.candidates {
            println!(
                "  {:<16} score {:.3} quality {:.3} match {:.2} valid {:.2} cover {:.2}",
                c.format.id, c.score, c.quality, c.match_rate, c.field_validity, c.coverage
            );
        }
    }
}
