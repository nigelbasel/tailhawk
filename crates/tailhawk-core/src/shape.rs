//! Text shaping — grapheme cluster to glyph ids. The bridge between V2 and V3.
//!
//! [`crate::cell`] decides how many columns a grapheme cluster occupies and [`crate::glyphs`] takes
//! a glyph id and returns a quad. **Nothing joined the two**, and the one-code-point-one-glyph
//! mapping the V2 tests used is wrong for every script `SPEC.md` §3.3 names: a Devanagari cluster
//! reorders and attaches marks, an Arabic letter picks a different glyph depending on what is beside
//! it, and a ZWJ emoji sequence is one cluster drawing one glyph out of six code points. This module
//! is the missing piece, and its output — **per-cluster glyph ranges** — is the currency the grid
//! consumes.
//!
//! **It holds no device and no window**, exactly like [`crate::raster`], which is what makes all of
//! it testable in-process. `IDWriteTextAnalyzer` is pure text machinery; the D3D device is not
//! involved until the quads are drawn.
//!
//! ## The call sequence, and the three things about it that bite
//!
//! `AnalyzeScript` → `GetGlyphs` → `GetGlyphPlacements`, with `AnalyzeBidi` alongside the first.
//! `AnalyzeScript` does not *return* anything: it calls back into an [`IDWriteTextAnalysisSource`]
//! we supply for the text and an [`IDWriteTextAnalysisSink`] we supply for the runs, which is why
//! this module authors two COM objects.
//!
//! - **Every position DirectWrite speaks is a UTF-16 code unit.** `cell.rs` speaks UTF-8 byte
//!   offsets. Getting that mapping wrong is silent: an astral-plane emoji is one `char` and one
//!   cluster but **two** UTF-16 code units and four UTF-8 bytes, so an off-by-one lands on a
//!   surrogate half and shapes garbage rather than failing. Both directions are carried explicitly
//!   in [`ClusterSpan`] rather than recomputed at each use.
//! - **`GetGlyphs` has no guaranteed output size.** The documented estimate is `3n/2 + 16`, and it
//!   is an estimate: a run that needs more fails with `HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)`
//!   and must be retried with a larger buffer. That is a normal outcome on decomposed Indic text,
//!   not an error condition.
//! - **`GetGlyphs` returns glyphs in *logical* order, even with `isRightToLeft = true`.** That is
//!   the opposite of the obvious guess, and it was measured rather than recalled — see
//!   `directwrite_returns_glyphs_in_logical_order`, which uses alef's one-sided joining to tell the
//!   two orders apart. Visual reordering is the drawing code's job, not this module's.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use unicode_segmentation::UnicodeSegmentation;
use windows::core::{implement, Interface, HRESULT, PCWSTR};
use windows::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory2, IDWriteNumberSubstitution, IDWriteTextAnalysisSink,
    IDWriteTextAnalysisSink_Impl, IDWriteTextAnalysisSource, IDWriteTextAnalysisSource_Impl,
    IDWriteTextAnalyzer, DWRITE_FACTORY_TYPE_SHARED, DWRITE_GLYPH_OFFSET, DWRITE_LINE_BREAKPOINT,
    DWRITE_READING_DIRECTION, DWRITE_READING_DIRECTION_LEFT_TO_RIGHT, DWRITE_SCRIPT_ANALYSIS,
    DWRITE_SHAPING_GLYPH_PROPERTIES, DWRITE_SHAPING_TEXT_PROPERTIES,
};

use crate::atlas::GlyphId;
use crate::raster::Face;
use crate::{Error, Result};

/// The locale handed to `GetGlyphs`. It selects language-specific forms within one script — Serbian
/// italic Cyrillic, Turkish `i`, and the several `locl` features real fonts carry.
///
/// Fixed for now, and that is a known limitation rather than a decision: a log file carries no
/// language tag, and guessing one from the bytes would be a worse answer than a stable one.
const LOCALE: &str = "en-us";

/// `GetGlyphs`' documented per-run buffer estimate: `3n/2 + 16` glyphs for `n` UTF-16 code units.
///
/// **Not a bound.** Decomposition can exceed it, which is what the retry loop is for.
fn estimate(units: usize) -> usize {
    units * 3 / 2 + 16
}

/// How many times the buffer may double before the run is given up on.
///
/// Four doublings is 16x the estimate — about 24 glyphs per code unit — which no real shaping
/// produces. A loop with no bound would answer a driver bug by exhausting memory.
const MAX_RETRIES: u32 = 4;

/// `HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER)`.
///
/// Composed from the binding's own `ERROR_INSUFFICIENT_BUFFER` rather than written as `0x8007007A`
/// from memory. V1 shipped a from-memory `D3DDDIERR_*` constant that did not exist in either
/// installed SDK, and it was only caught because someone went looking.
fn insufficient_buffer() -> HRESULT {
    const FACILITY_WIN32: u32 = 7;
    HRESULT(((ERROR_INSUFFICIENT_BUFFER.0 & 0xFFFF) | (FACILITY_WIN32 << 16) | 0x8000_0000) as i32)
}

/// One grapheme cluster, in both coordinate systems at once.
///
/// The UTF-16 half exists because DirectWrite counts in UTF-16 code units and everything else in
/// this crate counts in UTF-8 bytes. Carrying both is what stops the conversion being re-derived —
/// and re-derived differently — at each use.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClusterSpan {
    /// Byte offset of the cluster within the shaped line.
    pub byte: usize,
    /// Byte length of the cluster.
    pub byte_len: usize,
    /// UTF-16 code-unit offset of the cluster.
    pub unit: usize,
    /// UTF-16 code-unit length of the cluster.
    pub unit_len: usize,
}

/// A per-glyph placement adjustment, in device pixels at the shaped em size.
///
/// Non-zero for attached marks — a Devanagari vowel sign or an Arabic dot sits relative to the glyph
/// it belongs to, not on the pen position. `SPEC.md` §3.3 gives the *cell grid* the final say on
/// where a cluster starts; these move glyphs **within** the cluster.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct GlyphOffset {
    pub advance: f32,
    pub ascender: f32,
}

/// One grapheme cluster and the glyphs that draw it.
///
/// **`glyph_count` may be zero.** A ligature covers several clusters with one glyph, and it is
/// reported against the first of them; the rest genuinely draw nothing of their own. That is a real
/// state, not a failure — a grid that puts one glyph in one cell needs to *know* two clusters merged.
///
/// It is decided by whether any of the cluster's positions **opens** a DirectWrite cluster, not by
/// whether its first position was shared. The weaker test dropped glyphs; see [`glyph_range`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClusterGlyphs {
    pub span: ClusterSpan,
    /// Index into [`Shaped::glyphs`] of this cluster's first glyph.
    pub first_glyph: usize,
    /// How many consecutive glyphs draw it.
    pub glyph_count: usize,
}

/// One shaped run, and the resolved bidi level that decides where it is painted.
///
/// **The level is kept as a number, not as a `right_to_left` flag.** UAX #9's reordering needs the
/// magnitude: an odd/even bit cannot distinguish a left-to-right phrase embedded in a right-to-left
/// sentence (level 2 inside level 1) from an unrelated left-to-right run at level 0, and the two are
/// painted in different places. `shape.rs` itself only ever asks whether the level is odd, which is
/// how the flag survived until [`crate::bidi`] needed the rest of it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ShapedRun {
    /// The resolved bidi level, from `AnalyzeBidi`. Odd is right-to-left.
    pub level: u8,
    /// Index into [`Shaped::glyphs`] of this run's first glyph.
    pub first_glyph: usize,
    /// How many consecutive glyphs belong to it.
    pub glyph_count: usize,
}

/// The result of shaping one line.
///
/// `glyphs`, `advances` and `offsets` are parallel arrays; `clusters` is in logical order and
/// indexes into them.
///
/// **The glyph arrays are in logical order too, including for right-to-left runs** — that was
/// measured, not assumed, and it is the opposite of the obvious guess. See
/// `directwrite_returns_glyphs_in_logical_order`. [`Shaped::visual_glyphs`] is the reordering.
#[derive(Clone, Debug, Default)]
pub struct Shaped {
    pub glyphs: Vec<GlyphId>,
    pub advances: Vec<f32>,
    pub offsets: Vec<GlyphOffset>,
    pub clusters: Vec<ClusterGlyphs>,
    /// The runs, in **logical** order, each with the level [`Shaped::visual_glyphs`] reorders by.
    pub runs: Vec<ShapedRun>,
}

impl Shaped {
    /// The glyphs that draw one cluster. Empty when the cluster was absorbed into a ligature.
    pub fn glyphs_of(&self, cluster: &ClusterGlyphs) -> &[GlyphId] {
        &self.glyphs[cluster.first_glyph..cluster.first_glyph + cluster.glyph_count]
    }

    /// Glyph indices in the order they are painted, left to right.
    ///
    /// **The whole reason this exists is that `GetGlyphs` does not do it.** It returns a
    /// right-to-left run's glyphs in *logical* order, which was measured rather than assumed
    /// (`directwrite_returns_glyphs_in_logical_order`), so painting `self.glyphs` in array order
    /// draws Arabic and Hebrew backwards.
    ///
    /// **One level per glyph, not one per run, and that is not an optimisation left on the table.**
    /// Reordering runs alone puts the runs in the right places and leaves the glyphs *inside* a
    /// right-to-left run still in logical order — the bug moves from the line to the word. Expanding
    /// each run's level across its glyphs makes UAX #9's own reversal do both jobs in one pass,
    /// because reversing a window at level ≥ N reverses the items within it as well as the blocks.
    ///
    /// The identity for a line with no right-to-left content, which is almost every line in almost
    /// every log.
    pub fn visual_glyphs(&self) -> Vec<usize> {
        let mut levels = vec![0u8; self.glyphs.len()];
        for run in &self.runs {
            let end = (run.first_glyph + run.glyph_count).min(levels.len());
            levels[run.first_glyph.min(end)..end].fill(run.level);
        }
        crate::bidi::visual_order(&levels)
    }

    /// The cluster containing a byte offset, if any.
    pub fn cluster_at_byte(&self, byte: usize) -> Option<&ClusterGlyphs> {
        self.clusters
            .iter()
            .find(|c| byte >= c.span.byte && byte < c.span.byte + c.span.byte_len)
    }
}

/// A stretch of text with one script and one bidi level — the unit `GetGlyphs` shapes.
#[derive(Copy, Clone, Debug)]
struct Run {
    unit: usize,
    unit_len: usize,
    script: DWRITE_SCRIPT_ANALYSIS,
    level: u8,
}

impl Run {
    fn right_to_left(&self) -> bool {
        self.level % 2 == 1
    }
}

/// What the sink collects. Shared with the COM object by `Rc` rather than read back out of it,
/// because reaching from a COM interface pointer to the Rust value behind it is a great deal of
/// ceremony for something a clone of an `Rc` answers directly.
#[derive(Default)]
struct Collected {
    scripts: Vec<(u32, u32, DWRITE_SCRIPT_ANALYSIS)>,
    levels: Vec<(u32, u32, u8)>,
}

/// The text `AnalyzeScript` reads and the runs it reports, as one COM object implementing both
/// halves.
///
/// One object rather than two because the analyzer is handed both and they describe the same text;
/// splitting them would mean keeping two copies of it in sync.
#[implement(IDWriteTextAnalysisSource, IDWriteTextAnalysisSink)]
struct Analysis {
    text: Vec<u16>,
    locale: Vec<u16>,
    collected: Rc<RefCell<Collected>>,
}

/// **Every one of these returns `S_OK`, including the ones this module does not use.**
///
/// Measured, not assumed: instrumenting the five callbacks showed `AnalyzeScript` and `AnalyzeBidi`
/// between them calling `GetTextAtPosition`, **`GetLocaleName` and `GetParagraphReadingDirection`**
/// — and not `GetTextBeforePosition` or `GetNumberSubstitution`. Returning `E_NOTIMPL` from
/// `GetLocaleName`, on the reasonable-sounding grounds that a log file has no locale, **fails the
/// whole analysis and took all nine DirectWrite tests with it.** An `Err` from a callback is not
/// read as "I have nothing to say"; it is read as a failure of the call that asked.
///
/// So a constant plus `Ok(())` is the answer, and the two callbacks that were *not* observed being
/// called get the same treatment — they belong to the other `Analyze*` methods, which this module
/// may yet need.
impl IDWriteTextAnalysisSource_Impl for Analysis_Impl {
    /// The **remainder** of the text from `position` — not the whole block, and not one character.
    /// A source that returns one character at a time makes the analyzer walk the text one call per
    /// code unit and still works, which is why getting this wrong shows up as a performance
    /// mystery rather than a wrong answer.
    fn GetTextAtPosition(
        &self,
        position: u32,
        text: *mut *mut u16,
        length: *mut u32,
    ) -> windows::core::Result<()> {
        let position = position as usize;
        unsafe {
            if position >= self.text.len() {
                // NULL means end of text. Not an error.
                *text = std::ptr::null_mut();
                *length = 0;
            } else {
                *text = self.text.as_ptr().add(position) as *mut u16;
                *length = (self.text.len() - position) as u32;
            }
        }
        Ok(())
    }

    /// The block *ending* at `position` — so the text before it, not after.
    fn GetTextBeforePosition(
        &self,
        position: u32,
        text: *mut *mut u16,
        length: *mut u32,
    ) -> windows::core::Result<()> {
        let position = position as usize;
        unsafe {
            if position == 0 || position > self.text.len() {
                *text = std::ptr::null_mut();
                *length = 0;
            } else {
                *text = self.text.as_ptr() as *mut u16;
                *length = position as u32;
            }
        }
        Ok(())
    }

    /// The *paragraph* direction, which is not the same question as a run's resolved bidi level.
    /// A log line is a left-to-right paragraph that may contain right-to-left runs; `AnalyzeBidi`
    /// resolves those, and this is only the base it resolves against.
    fn GetParagraphReadingDirection(&self) -> DWRITE_READING_DIRECTION {
        DWRITE_READING_DIRECTION_LEFT_TO_RIGHT
    }

    fn GetLocaleName(
        &self,
        position: u32,
        length: *mut u32,
        locale: *mut *mut u16,
    ) -> windows::core::Result<()> {
        unsafe {
            *length = (self.text.len() as u32).saturating_sub(position);
            *locale = self.locale.as_ptr() as *mut u16;
        }
        Ok(())
    }

    fn GetNumberSubstitution(
        &self,
        position: u32,
        length: *mut u32,
        substitution: *mut Option<IDWriteNumberSubstitution>,
    ) -> windows::core::Result<()> {
        unsafe {
            *length = (self.text.len() as u32).saturating_sub(position);
            *substitution = None;
        }
        Ok(())
    }
}

impl IDWriteTextAnalysisSink_Impl for Analysis_Impl {
    fn SetScriptAnalysis(
        &self,
        position: u32,
        length: u32,
        script: *const DWRITE_SCRIPT_ANALYSIS,
    ) -> windows::core::Result<()> {
        self.collected
            .borrow_mut()
            .scripts
            .push((position, length, unsafe { *script }));
        Ok(())
    }

    fn SetLineBreakpoints(
        &self,
        _position: u32,
        _length: u32,
        _breakpoints: *const DWRITE_LINE_BREAKPOINT,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    /// The **resolved** level is the one that matters; the explicit level is what the embedding
    /// controls asked for before resolution. An odd resolved level is a right-to-left run.
    fn SetBidiLevel(
        &self,
        position: u32,
        length: u32,
        _explicit: u8,
        resolved: u8,
    ) -> windows::core::Result<()> {
        self.collected
            .borrow_mut()
            .levels
            .push((position, length, resolved));
        Ok(())
    }

    fn SetNumberSubstitution(
        &self,
        _position: u32,
        _length: u32,
        _substitution: Option<&IDWriteNumberSubstitution>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

/// Every grapheme cluster of `line`, in both coordinate systems.
///
/// UTF-16 length is additive over concatenation, so accumulating each cluster's own
/// `encode_utf16().count()` gives the same offsets as encoding the whole line — which is what makes
/// the two halves of a [`ClusterSpan`] describe the same cluster.
fn cluster_spans(line: &str) -> Vec<ClusterSpan> {
    let mut unit = 0usize;
    line.grapheme_indices(true)
        .map(|(byte, cluster)| {
            let unit_len = cluster.encode_utf16().count();
            let span = ClusterSpan {
                byte,
                byte_len: cluster.len(),
                unit,
                unit_len,
            };
            unit += unit_len;
            span
        })
        .collect()
}

/// The glyphs belonging to one cluster, given the `clusterMap` of the run it sits in.
///
/// **A free function so it can be tested on synthetic maps**, which is the only way to cover
/// ligature and right-to-left shapes on a machine whose fonts may have neither. `raster.rs`'s
/// `tighten` was extracted for the same reason.
///
/// `clusterMap` maps each UTF-16 position in the run to the index of the **first** glyph drawing
/// the cluster that position belongs to, and every position of one DirectWrite cluster maps to the
/// same index. So the cluster's own entry is its first glyph, and the question is only where it
/// ends: at whichever neighbouring cluster sits *after* it in the glyph array.
///
/// **Which neighbour that is, is deliberately not assumed.** The map is monotonic but its direction
/// is the shaper's business, so this takes whichever adjacent distinct value is greater than
/// `first` — the next one in a rising map, the previous one in a falling one — and the end of the
/// array when there is none. That makes the function correct under either convention rather than
/// under the one this machine happens to use, which is worth having because the convention was
/// **measured and came out the opposite way from the obvious guess**: see
/// `directwrite_returns_glyphs_in_logical_order`.
///
/// Returns `(first, count)`. **A count of zero means every one of the cluster's positions belongs to
/// a DirectWrite cluster that started before it** — it was swallowed by a preceding ligature and
/// draws nothing of its own. Reported rather than smoothed over: a grid that puts one cluster in one
/// cell needs to know two of them merged.
///
/// ## Ownership is decided by where a DirectWrite cluster *starts*
///
/// The two clusterings do not nest. DirectWrite's boundaries and `unicode-segmentation`'s can
/// interleave, so one grapheme cluster's positions can span the tail of one DirectWrite cluster and
/// the whole of the next. The rule is that a grapheme cluster owns every DirectWrite cluster whose
/// **first** position falls inside it, and a position starts one when it is position zero or its map
/// entry differs from the position before.
///
/// **Asking only whether the cluster's own first position was shared — which is what this did — drops
/// glyphs.** With `map = [0, 0, 1]` and grapheme clusters `(0..1)` and `(1..3)`, the second cluster's
/// first position shares glyph 0 with the first cluster, so it was declared absorbed and returned
/// zero glyphs; but its *second* position starts a new DirectWrite cluster, and glyph 1 was then
/// claimed by nobody and never drawn. `\u{200B}\u{FE0F}\u{E0067}` is a real line that does this — and
/// the glyph it dropped belonged to a tag character, the hidden-text vector `SPEC.md` §13.4 names,
/// which §5.6 requires never be silently discarded.
fn glyph_range(
    cluster_map: &[u16],
    glyph_count: usize,
    start: usize,
    len: usize,
) -> (usize, usize) {
    if len == 0 || start >= cluster_map.len() {
        return (glyph_count, 0);
    }
    let end = (start + len).min(cluster_map.len());
    let starts_a_run = |p: usize| p == 0 || cluster_map[p] != cluster_map[p - 1];

    // The first position of the cluster that opens a DirectWrite cluster of its own. Without one,
    // every glyph the cluster touches is its predecessor's.
    let Some(opening) = (start..end).find(|p| starts_a_run(*p)) else {
        return ((cluster_map[start] as usize).min(glyph_count), 0);
    };
    let first = cluster_map[opening] as usize;

    // The limit is the neighbouring distinct entry that sits *after* this cluster in the glyph
    // array. Forward has to skip equal entries — a cluster several code units long maps all of them
    // to one glyph — and is measured from the cluster's last entry, which may differ from its first.
    let last = cluster_map[end - 1] as usize;
    let mut ahead = end;
    while ahead < cluster_map.len() && cluster_map[ahead] as usize == last {
        ahead += 1;
    }
    let neighbours = [
        (ahead < cluster_map.len()).then(|| cluster_map[ahead] as usize),
        (start > 0).then(|| cluster_map[start - 1] as usize),
    ];

    let limit = neighbours
        .into_iter()
        .flatten()
        .filter(|n| *n > first)
        .min()
        .unwrap_or(glyph_count);

    let first = first.min(glyph_count);
    (first, limit.min(glyph_count).saturating_sub(first))
}

/// Moves a run boundary onto the nearest grapheme cluster boundary, **forwards**.
///
/// Script and bidi boundaries are found on the raw text and need not agree with grapheme cluster
/// boundaries — a combining mark can carry a different script from its base, and an isolate control
/// is its own bidi run inside somebody's cluster. **A cluster is indivisible**: shaping half of one
/// in each of two runs would give a mark no base to attach to. So a boundary landing inside a cluster
/// has to move, and the cluster goes wholesale to the run on one side or the other.
///
/// **It moves to the cluster's end, not its start, and that is the whole point.** Moving backwards
/// looks equivalent — the straddling cluster simply joins the later run instead of the earlier one —
/// but the cluster's start is very often *already* a boundary, and then the moved boundary lands on
/// an existing one and **disappears from the set entirely**. The run does not merely absorb one
/// cluster; it swallows everything up to the next surviving boundary.
///
/// The cost of getting this wrong is not subtle. `"بि"` is one grapheme cluster — `U+093F` is a
/// spacing mark, so it joins the Arabic letter before it — and it straddles the Arabic/Devanagari
/// script boundary. Snapping backwards deleted that boundary, so `"بिक्षि tail"` shaped its entire
/// Devanagari word as right-to-left Arabic: the conjunct ligature lost, the matra not reordered,
/// four glyphs where two belong and the cluster **77% wider**, which is column drift against §3.3's
/// own acceptance test. It reaches past the script too — `"GET /a بिक्षि 200"` shaped the trailing
/// space and `200` as Arabic. Six attacker-supplied bytes changing the rendering of the rest of the
/// line is exactly what §13.4 calls a real defect.
fn snap_to_cluster(unit: usize, spans: &[ClusterSpan]) -> usize {
    match spans.binary_search_by_key(&unit, |s| s.unit) {
        Ok(_) => unit,
        Err(0) => 0,
        Err(i) => spans[i - 1].unit + spans[i - 1].unit_len,
    }
}

/// One line in both of the forms shaping needs it in at once: the UTF-16 buffer DirectWrite reads,
/// and the grapheme cluster boundaries every result is reported against. They are always derived
/// from the same `&str` and are meaningless apart, so they travel together.
struct Line<'a> {
    text: &'a [u16],
    spans: &'a [ClusterSpan],
}

pub struct Shaper {
    analyzer: IDWriteTextAnalyzer,
    locale: Vec<u16>,
}

impl Shaper {
    pub fn new() -> Result<Self> {
        let factory: IDWriteFactory2 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(Self {
            analyzer: unsafe { factory.CreateTextAnalyzer()? },
            locale: wide(LOCALE),
        })
    }

    /// Shapes one line against one face.
    ///
    /// **One face.** Font fallback (§3.3) is a separate problem: a codepoint the face lacks shapes
    /// to glyph 0, `.notdef`, and it is the caller's job to notice and try the next face — exactly
    /// as [`Face::glyph_indices`] already behaves. Doing fallback inside shaping would mean one
    /// cluster's glyphs coming from two faces, and [`crate::atlas::GlyphKey`] is keyed by face for
    /// precisely that reason.
    pub fn shape(&self, face: &Face, line: &str, px_per_em: u16) -> Result<Shaped> {
        self.shape_from(face, line, px_per_em, estimate)
    }

    /// [`shape`](Self::shape) with the first buffer guess supplied, so the retry loop can be driven.
    ///
    /// The loop is otherwise dead code on every input a test can construct: the documented estimate
    /// is generous enough that reaching `ERROR_INSUFFICIENT_BUFFER` on purpose would need a font
    /// whose shaping explodes, and there is no arranging that. Starving the first guess instead
    /// exercises the same path — the same error, the same growth, the same second call — which is
    /// the trick `rasterise_in_batches` already uses to make batch size testable.
    fn shape_from(
        &self,
        face: &Face,
        line: &str,
        px_per_em: u16,
        guess: fn(usize) -> usize,
    ) -> Result<Shaped> {
        let spans = cluster_spans(line);
        if spans.is_empty() {
            return Ok(Shaped::default());
        }
        let text: Vec<u16> = line.encode_utf16().collect();
        // Every position DirectWrite takes is a `u32`. A line past that would be analysed as a
        // truncated prefix and the clusters beyond it would fall in no run and vanish from the
        // result — a silent disagreement with `cell.rs` about how many clusters a line has, which
        // the grid assumes cannot happen. Refusing is the honest answer; §10.3's long-line handling
        // is a different problem from pretending this one does not exist.
        if text.len() > u32::MAX as usize {
            return Err(Error(format!(
                "a line of {} UTF-16 code units is past what DirectWrite can analyse",
                text.len()
            )));
        }
        let line = Line {
            text: &text,
            spans: &spans,
        };

        let mut out = Shaped {
            clusters: Vec::with_capacity(spans.len()),
            ..Shaped::default()
        };
        for run in self.runs(&line)? {
            self.shape_run(face, &line, run, px_per_em, guess, &mut out)?;
        }
        Ok(out)
    }

    /// Splits the line into stretches of one script and one bidi level.
    fn runs(&self, line: &Line) -> Result<Vec<Run>> {
        let collected = Rc::new(RefCell::new(Collected::default()));
        let source: IDWriteTextAnalysisSource = Analysis {
            text: line.text.to_vec(),
            locale: self.locale.clone(),
            collected: collected.clone(),
        }
        .into();
        let sink: IDWriteTextAnalysisSink = source.cast()?;

        let len = line.text.len() as u32;
        unsafe {
            self.analyzer.AnalyzeScript(&source, 0, len, &sink)?;
            self.analyzer.AnalyzeBidi(&source, 0, len, &sink)?;
        }
        let collected = collected.borrow();

        // A boundary from either analysis starts a run, moved onto a cluster boundary first.
        let mut cuts = BTreeSet::from([0usize, line.text.len()]);
        for (position, _, _) in &collected.scripts {
            cuts.insert(snap_to_cluster(*position as usize, line.spans));
        }
        for (position, _, _) in &collected.levels {
            cuts.insert(snap_to_cluster(*position as usize, line.spans));
        }

        // `cuts` is a set, so consecutive pairs are strictly increasing and every run is non-empty.
        let cuts: Vec<usize> = cuts.into_iter().collect();
        Ok(cuts
            .windows(2)
            .map(|w| Run {
                unit: w[0],
                unit_len: w[1] - w[0],
                script: at(&collected.scripts, w[0]).unwrap_or_default(),
                level: at(&collected.levels, w[0]).unwrap_or(0),
            })
            .collect())
    }

    fn shape_run(
        &self,
        face: &Face,
        line: &Line,
        run: Run,
        px_per_em: u16,
        guess: fn(usize) -> usize,
        out: &mut Shaped,
    ) -> Result<()> {
        let len = run.unit_len;
        // Not NUL-terminated, and it does not need to be: `GetGlyphs` takes the length separately,
        // which is what lets a run start in the middle of the line's buffer.
        let start = PCWSTR(line.text[run.unit..].as_ptr());
        let sideways = false;
        let rtl = run.right_to_left();
        let locale = PCWSTR(self.locale.as_ptr());

        let mut cluster_map = vec![0u16; len];
        let mut text_props = vec![DWRITE_SHAPING_TEXT_PROPERTIES::default(); len];
        let mut capacity = guess(len).max(1);
        let mut glyphs;
        let mut glyph_props;
        let mut count = 0u32;

        let mut retries = 0;
        loop {
            glyphs = vec![0u16; capacity];
            glyph_props = vec![DWRITE_SHAPING_GLYPH_PROPERTIES::default(); capacity];
            let attempt = unsafe {
                self.analyzer.GetGlyphs(
                    start,
                    len as u32,
                    face.font_face(),
                    sideways,
                    rtl,
                    &run.script,
                    locale,
                    None,
                    None,
                    None,
                    0,
                    capacity as u32,
                    cluster_map.as_mut_ptr(),
                    text_props.as_mut_ptr(),
                    glyphs.as_mut_ptr(),
                    glyph_props.as_mut_ptr(),
                    &mut count,
                )
            };
            match attempt {
                Ok(()) => break,
                // The documented estimate is not a bound. Decomposition can exceed it, and this is
                // how DirectWrite says so.
                Err(e) if e.code() == insufficient_buffer() && retries < MAX_RETRIES => {
                    capacity *= 2;
                    retries += 1;
                }
                Err(e) => return Err(Error(format!("GetGlyphs: {e}"))),
            }
        }
        let count = count as usize;
        glyphs.truncate(count);

        let mut advances = vec![0f32; count];
        let mut offsets = vec![DWRITE_GLYPH_OFFSET::default(); count];
        unsafe {
            self.analyzer.GetGlyphPlacements(
                start,
                cluster_map.as_ptr(),
                text_props.as_mut_ptr(),
                len as u32,
                glyphs.as_ptr(),
                glyph_props.as_ptr(),
                count as u32,
                face.font_face(),
                px_per_em as f32,
                sideways,
                rtl,
                &run.script,
                locale,
                None,
                None,
                0,
                advances.as_mut_ptr(),
                offsets.as_mut_ptr(),
            )?
        };

        let base = out.glyphs.len();
        out.runs.push(ShapedRun {
            level: run.level,
            first_glyph: base,
            glyph_count: count,
        });
        out.glyphs.extend_from_slice(&glyphs);
        out.advances.extend_from_slice(&advances);
        out.offsets.extend(offsets.iter().map(|o| GlyphOffset {
            advance: o.advanceOffset,
            ascender: o.ascenderOffset,
        }));

        for span in line
            .spans
            .iter()
            .filter(|s| s.unit >= run.unit && s.unit < run.unit + len)
        {
            let (first, glyph_count) =
                glyph_range(&cluster_map, count, span.unit - run.unit, span.unit_len);
            out.clusters.push(ClusterGlyphs {
                span: *span,
                first_glyph: base + first,
                glyph_count,
            });
        }
        Ok(())
    }
}

/// The value reported for the analysis run containing `unit`.
fn at<T: Copy>(runs: &[(u32, u32, T)], unit: usize) -> Option<T> {
    runs.iter()
        .find(|(position, length, _)| {
            unit >= *position as usize && unit < (*position + *length) as usize
        })
        .map(|(_, _, value)| *value)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EM: u16 = 14;

    /// Latin faces that ship with Windows. Consolas has been in the box since Vista.
    const LATIN: &[&str] = &["Cascadia Mono", "Consolas", "Courier New", "Segoe UI"];
    /// Windows' Indic UI face, plus the older one it replaced.
    const DEVANAGARI: &[&str] = &["Nirmala UI", "Mangal"];
    /// Faces with Arabic. Segoe UI carries it; Tahoma and Arial are the long-standing fallbacks.
    const ARABIC: &[&str] = &["Segoe UI", "Tahoma", "Arial"];

    fn shaper_and_face(what: &str, candidates: &[&str]) -> Option<(Shaper, Face)> {
        let shaper = match Shaper::new() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("SKIPPED {what}: no DirectWrite text analyzer ({e})");
                return None;
            }
        };
        let rasteriser = match crate::raster::Rasteriser::new() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("SKIPPED {what}: no DirectWrite factory ({e})");
                return None;
            }
        };
        match rasteriser.resolve(candidates) {
            Ok(face) => Some((shaper, face)),
            Err(e) => {
                eprintln!("SKIPPED {what}: none of {candidates:?} is installed ({e})");
                None
            }
        }
    }

    /// True when the face actually covers the text — shaping a script the face lacks gives
    /// `.notdef` for every glyph, and asserting anything about those is asserting about nothing.
    fn covered(face: &Face, text: &str) -> bool {
        let codepoints: Vec<u32> = text.chars().map(|c| c as u32).collect();
        face.glyph_indices(&codepoints).iter().all(|g| *g != 0)
    }

    // ---- the pure parts, which need no font at all ----

    #[test]
    fn a_cluster_span_carries_both_coordinate_systems() {
        // 'e' + combining acute is one cluster of 3 bytes and 2 UTF-16 units; the astral emoji is
        // one cluster of 4 bytes and 2 units. Byte length and unit length agree on neither.
        let spans = cluster_spans("ae\u{301}\u{1F600}b");
        assert_eq!(
            spans,
            vec![
                ClusterSpan {
                    byte: 0,
                    byte_len: 1,
                    unit: 0,
                    unit_len: 1
                },
                ClusterSpan {
                    byte: 1,
                    byte_len: 3,
                    unit: 1,
                    unit_len: 2
                },
                ClusterSpan {
                    byte: 4,
                    byte_len: 4,
                    unit: 3,
                    unit_len: 2
                },
                ClusterSpan {
                    byte: 8,
                    byte_len: 1,
                    unit: 5,
                    unit_len: 1
                },
            ]
        );
    }

    #[test]
    fn cluster_spans_tile_the_line_exactly() {
        for line in [
            "",
            "hello",
            "\u{905}\u{93F}x",
            "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{202E}abc",
            "\r\n",
        ] {
            let spans = cluster_spans(line);
            let mut byte = 0;
            let mut unit = 0;
            for span in &spans {
                assert_eq!(span.byte, byte, "{line:?} has a byte gap");
                assert_eq!(span.unit, unit, "{line:?} has a unit gap");
                byte += span.byte_len;
                unit += span.unit_len;
            }
            assert_eq!(byte, line.len());
            assert_eq!(unit, line.encode_utf16().count());
        }
    }

    /// A rising `clusterMap`: a cluster's glyphs run from its own entry to the next cluster's.
    #[test]
    fn a_cluster_takes_the_glyphs_up_to_the_next_one() {
        // 3 positions, 4 glyphs: the middle cluster expanded into two.
        let map = [0u16, 1, 3];
        assert_eq!(glyph_range(&map, 4, 0, 1), (0, 1));
        assert_eq!(glyph_range(&map, 4, 1, 1), (1, 2));
        assert_eq!(glyph_range(&map, 4, 2, 1), (3, 1));
    }

    /// A falling `clusterMap` — the shape a visually-ordered right-to-left run would have. The same
    /// function must handle it, because which convention DirectWrite uses is measured rather than
    /// assumed, and a map read the wrong way round gives every cluster an empty range.
    #[test]
    fn a_falling_cluster_map_is_read_the_other_way_round() {
        // Logical A B C, glyphs laid out C B A.
        let map = [2u16, 1, 0];
        assert_eq!(glyph_range(&map, 3, 0, 1), (2, 1));
        assert_eq!(glyph_range(&map, 3, 1, 1), (1, 1));
        assert_eq!(glyph_range(&map, 3, 2, 1), (0, 1));

        // And with the first logical cluster expanding into two glyphs: [B, A0, A1].
        let map = [1u16, 0];
        assert_eq!(glyph_range(&map, 3, 0, 1), (1, 2));
        assert_eq!(glyph_range(&map, 3, 1, 1), (0, 1));
    }

    /// Two clusters, one glyph: the **first** owns it and the second honestly reports zero rather
    /// than borrowing it. Both clusters claiming the glyph would draw it twice; neither claiming it
    /// would drop a character.
    #[test]
    fn a_ligature_belongs_to_the_first_of_the_clusters_it_covers() {
        let map = [0u16, 0];
        assert_eq!(glyph_range(&map, 1, 0, 1), (0, 1));
        assert_eq!(glyph_range(&map, 1, 1, 1), (0, 0));

        // Three clusters, one glyph, and a fourth standing alone after them.
        let map = [0u16, 0, 0, 1];
        assert_eq!(glyph_range(&map, 2, 0, 1), (0, 1));
        assert_eq!(glyph_range(&map, 2, 1, 1), (0, 0));
        assert_eq!(glyph_range(&map, 2, 2, 1), (0, 0));
        assert_eq!(glyph_range(&map, 2, 3, 1), (1, 1));
    }

    /// **The dropped-glyph case, in the two shapes a review found it in.** A grapheme cluster whose
    /// *first* position was swallowed by the previous cluster's ligature, but whose later positions
    /// open a DirectWrite cluster of their own. Asking only about the first position calls the whole
    /// thing absorbed and the later glyph is claimed by nobody.
    #[test]
    fn a_cluster_that_starts_inside_a_ligature_still_owns_what_follows() {
        // Cluster (0..1) takes glyph 0; cluster (1..3) shares position 1 with it but opens a new
        // DirectWrite cluster at position 2, so glyph 1 is its own.
        let map = [0u16, 0, 1];
        assert_eq!(glyph_range(&map, 2, 0, 1), (0, 1));
        assert_eq!(glyph_range(&map, 2, 1, 2), (1, 1));

        // The mirror: the ligature is inside cluster (0..2), and cluster (2..3) is fully absorbed.
        let map = [0u16, 1, 1];
        assert_eq!(glyph_range(&map, 2, 0, 2), (0, 2));
        assert_eq!(glyph_range(&map, 2, 2, 1), (1, 0));
    }

    /// A cluster that is several code units long is still one entry's worth of glyphs — every
    /// position within it maps to the same first glyph.
    #[test]
    fn a_multi_unit_cluster_is_bounded_by_the_cluster_after_it() {
        // A surrogate pair then a letter: positions 0 and 1 are one cluster.
        let map = [0u16, 0, 2];
        assert_eq!(glyph_range(&map, 3, 0, 2), (0, 2));
        assert_eq!(glyph_range(&map, 3, 2, 1), (2, 1));
    }

    /// **A grapheme cluster DirectWrite does not agree is one cluster.** Our boundaries come from
    /// `unicode-segmentation` and DirectWrite has its own opinion; where a font lacks the ligature
    /// for a ZWJ emoji sequence, one grapheme cluster comes back as several DirectWrite clusters
    /// with a *rising* map inside it. All of those glyphs belong to the one grapheme cluster, which
    /// is why the forward scan starts at the cluster's end rather than one past its start.
    ///
    /// This case exists because a negative control did not fire without it: moving the scan to
    /// `start + 1` broke nothing, and the fixtures were the reason, not the code.
    #[test]
    fn a_cluster_directwrite_splits_still_takes_all_of_its_glyphs() {
        // One grapheme cluster over positions 0..2, which DirectWrite split in two.
        let map = [0u16, 1, 2];
        assert_eq!(glyph_range(&map, 3, 0, 2), (0, 2));
        assert_eq!(glyph_range(&map, 3, 2, 1), (2, 1));
    }

    /// **Every glyph is claimed by exactly one cluster.** This is the property the grid depends on:
    /// a glyph claimed twice draws twice, and one claimed by nobody silently disappears.
    ///
    /// It is swept exhaustively over every monotonic `clusterMap` of up to four positions, rising
    /// and falling, crossed with **every way of partitioning those positions into grapheme
    /// clusters** — because the two clusterings do not nest, and the partition is the axis that
    /// matters. An earlier version of this test drove only one-code-unit clusters and passed while
    /// `glyph_range` dropped glyphs on `map = [0, 0, 1]` split as `(0..1)(1..3)`. Multi-unit clusters
    /// are the entire reason shaping exists — surrogate pairs, matras, ZWJ sequences — so a sweep
    /// that omits them is a source of false confidence rather than a check.
    #[test]
    fn every_glyph_is_claimed_by_exactly_one_cluster() {
        let mut checked = 0usize;
        for map in monotonic_maps(4) {
            let glyph_count = *map.iter().max().expect("non-empty") as usize + 1;
            for extra in 0..2 {
                let glyph_count = glyph_count + extra;
                for partition in partitions(map.len()) {
                    let mut claims = vec![0usize; glyph_count];
                    for (start, len) in &partition {
                        let (first, count) = glyph_range(&map, glyph_count, *start, *len);
                        assert!(
                            first + count <= glyph_count,
                            "map {map:?} cluster ({start},{len}) escapes {glyph_count} glyphs"
                        );
                        for c in claims.iter_mut().skip(first).take(count) {
                            *c += 1;
                        }
                    }
                    assert!(
                        claims.iter().all(|c| *c == 1),
                        "map {map:?} over {glyph_count} glyphs, clusters {partition:?}, \
                         claimed {claims:?}"
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 500, "the sweep should be broad, ran {checked}");
    }

    /// Every non-decreasing map starting at 0 with steps of 0, 1 or 2, plus each one's falling
    /// mirror — the shape a visually-ordered run would have.
    fn monotonic_maps(max_len: usize) -> Vec<Vec<u16>> {
        let mut out = Vec::new();
        for len in 1..=max_len {
            let mut rising: Vec<Vec<u16>> = vec![vec![0u16]];
            for _ in 1..len {
                rising = rising
                    .into_iter()
                    .flat_map(|m| {
                        let last = *m.last().expect("non-empty");
                        (0..=2u16).map(move |step| {
                            let mut next = m.clone();
                            next.push(last + step);
                            next
                        })
                    })
                    .collect();
            }
            for m in rising {
                let top = *m.iter().max().expect("non-empty");
                out.push(m.iter().rev().map(|v| top - v).collect());
                out.push(m);
            }
        }
        out
    }

    /// Every way of cutting `n` positions into contiguous non-empty clusters, as `(start, len)`.
    fn partitions(n: usize) -> Vec<Vec<(usize, usize)>> {
        (0..1u32 << (n.saturating_sub(1)))
            .map(|bits| {
                let mut out = Vec::new();
                let mut start = 0;
                for p in 1..n {
                    if bits & (1 << (p - 1)) != 0 {
                        out.push((start, p - start));
                        start = p;
                    }
                }
                out.push((start, n - start));
                out
            })
            .collect()
    }

    #[test]
    fn a_boundary_inside_a_cluster_moves_forward_to_its_end() {
        let spans = cluster_spans("a\u{1F600}b");
        // Units: 0 = 'a', 1..3 = the emoji, 3 = 'b', 4 = end.
        assert_eq!(snap_to_cluster(0, &spans), 0);
        assert_eq!(snap_to_cluster(1, &spans), 1);
        assert_eq!(
            snap_to_cluster(2, &spans),
            3,
            "a boundary on the low surrogate must not split the pair, and must not vanish either"
        );
        assert_eq!(snap_to_cluster(3, &spans), 3);
        assert_eq!(snap_to_cluster(9, &spans), 4);
    }

    /// **A boundary must survive being moved.** Snapping backwards lands an interior boundary on the
    /// cluster's start, which is usually already a cut — and a boundary that collapses onto an
    /// existing one is a boundary deleted, letting the earlier run swallow everything up to the next
    /// surviving cut. Snapping forwards cannot do that, because a cluster's end is a position no
    /// earlier cut can occupy.
    #[test]
    fn a_snapped_boundary_is_never_the_run_start_it_would_collapse_into() {
        for line in [
            "a\u{1F600}b",
            "\u{628}\u{93F}\u{915}",
            "\u{1F468}\u{200D}\u{1F469}x",
            "e\u{301}\u{302}f",
            "\r\na",
        ] {
            let spans = cluster_spans(line);
            for span in &spans {
                // Every position strictly inside a cluster must land past the cluster's own start,
                // which is the cut it would otherwise be swallowed by.
                for interior in span.unit + 1..span.unit + span.unit_len {
                    let snapped = snap_to_cluster(interior, &spans);
                    assert_eq!(
                        snapped,
                        span.unit + span.unit_len,
                        "{line:?}: the boundary at {interior} should land on the cluster's end"
                    );
                    assert!(
                        snapped > span.unit,
                        "{line:?}: the boundary at {interior} collapsed onto the cut at {}",
                        span.unit
                    );
                }
            }
        }
    }

    #[test]
    fn the_buffer_estimate_is_the_documented_one() {
        assert_eq!(estimate(0), 16);
        assert_eq!(estimate(8), 28);
    }

    #[test]
    fn the_retry_code_is_the_one_win32_actually_sends() {
        assert_eq!(insufficient_buffer(), HRESULT(0x8007_007Au32 as i32));
    }

    // ---- and the parts that need DirectWrite ----

    #[test]
    fn every_cluster_of_a_line_is_accounted_for() {
        let Some((shaper, face)) =
            shaper_and_face("every_cluster_of_a_line_is_accounted_for", LATIN)
        else {
            return;
        };
        let line = "2026-08-06T09:41:02Z WARN  pool exhausted (n=17)";
        let shaped = shaper.shape(&face, line, EM).expect("shape");

        let spans = cluster_spans(line);
        assert_eq!(
            shaped.clusters.len(),
            spans.len(),
            "shaping must report every cluster, in order"
        );
        for (got, want) in shaped.clusters.iter().zip(&spans) {
            assert_eq!(got.span, *want);
        }
        assert_eq!(shaped.advances.len(), shaped.glyphs.len());
        assert_eq!(shaped.offsets.len(), shaped.glyphs.len());
    }

    /// On plain ASCII, shaping and the naive one-codepoint-one-glyph map must agree. If they do not,
    /// something far more basic than shaping is wrong — and this is the test that keeps the *rest*
    /// of the suite's failures meaningful.
    #[test]
    fn ascii_shapes_to_exactly_what_the_cmap_says() {
        let Some((shaper, face)) =
            shaper_and_face("ascii_shapes_to_exactly_what_the_cmap_says", LATIN)
        else {
            return;
        };
        let line: String = (0x20u8..0x7F).map(|b| b as char).collect();
        let shaped = shaper.shape(&face, &line, EM).expect("shape");

        let direct = face.glyph_indices(&line.chars().map(|c| c as u32).collect::<Vec<_>>());
        assert_eq!(shaped.glyphs.len(), direct.len());
        assert_eq!(shaped.glyphs, direct);
        for (i, cluster) in shaped.clusters.iter().enumerate() {
            assert_eq!(
                shaped.glyphs_of(cluster),
                &direct[i..i + 1],
                "ASCII character {i} should draw as its own single glyph"
            );
        }
    }

    /// **The reason this module exists.** `कि` is one grapheme cluster, `क` + `ि`, and the vowel sign
    /// is written *before* the consonant even though it is stored after it. One code point to one
    /// glyph cannot express that; shaping can.
    #[test]
    fn a_devanagari_cluster_reorders_and_still_reports_as_one_cluster() {
        let Some((shaper, face)) = shaper_and_face(
            "a_devanagari_cluster_reorders_and_still_reports_as_one_cluster",
            DEVANAGARI,
        ) else {
            return;
        };
        let line = "\u{915}\u{93F}";
        if !covered(&face, line) {
            eprintln!(
                "SKIPPED a_devanagari_cluster_reorders...: {} lacks Devanagari",
                face.family
            );
            return;
        }
        let shaped = shaper.shape(&face, line, EM).expect("shape");

        assert_eq!(shaped.clusters.len(), 1, "क + ि is one grapheme cluster");
        let glyphs = shaped.glyphs_of(&shaped.clusters[0]);
        assert_eq!(glyphs.len(), 2, "and it draws as two glyphs");

        let direct = face.glyph_indices(&[0x915, 0x93F]);
        assert_ne!(
            glyphs,
            &direct[..],
            "if shaping returns the codepoints' own glyphs in their own order, nothing was shaped"
        );
        assert_eq!(
            glyphs,
            &[direct[1], direct[0]],
            "the vowel sign is drawn before the consonant it follows"
        );
    }

    /// An Arabic letter picks its glyph from what is beside it — isolated, initial, medial, final.
    /// `ب` in three positions of one word must give three different glyphs; the `.cmap` gives one.
    #[test]
    fn an_arabic_letter_shapes_differently_by_position() {
        let Some((shaper, face)) =
            shaper_and_face("an_arabic_letter_shapes_differently_by_position", ARABIC)
        else {
            return;
        };
        let beh = '\u{628}';
        if !covered(&face, "\u{628}") {
            eprintln!(
                "SKIPPED an_arabic_letter_shapes...: {} lacks Arabic",
                face.family
            );
            return;
        }

        let word: String = std::iter::repeat_n(beh, 3).collect();
        let shaped = shaper.shape(&face, &word, EM).expect("shape");
        assert_eq!(shaped.clusters.len(), 3);

        let forms: Vec<GlyphId> = shaped
            .clusters
            .iter()
            .flat_map(|c| shaped.glyphs_of(c).to_vec())
            .collect();
        assert_eq!(forms.len(), 3, "three letters, three glyphs");
        assert_eq!(
            forms
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            3,
            "initial, medial and final ب must be three different glyphs, not {forms:?}"
        );

        let isolated = face.glyph_indices(&[beh as u32])[0];
        assert!(
            !forms.contains(&isolated),
            "no letter in the middle of a word draws in its isolated form"
        );
    }

    /// **The measurement that decided how [`glyph_range`] reads the map, and it came out opposite to
    /// the obvious guess.** A right-to-left run draws right to left, so the natural assumption is
    /// that `GetGlyphs` hands back the array already reversed. It does not: with
    /// `isRightToLeft = true` the glyphs are still in **logical** order and `clusterMap` still
    /// rises. Reversal is the caller's job at draw time.
    ///
    /// The discriminator is alef, which joins to its right but not to its left, so `ابب` is
    /// unambiguously `[alef, beh-initial, beh-final]` logically and `[beh-final, beh-initial, alef]`
    /// visually. Alef's glyph is identifiable from the `.cmap`, so whichever end it turns up at
    /// names the convention — and it turns up first.
    ///
    /// If a future DirectWrite changes this, that is what this test is for. [`glyph_range`] survives
    /// the change because it reads the map's direction rather than assuming one; the *drawing* code
    /// will not.
    #[test]
    fn directwrite_returns_glyphs_in_logical_order() {
        let Some((shaper, face)) =
            shaper_and_face("directwrite_returns_glyphs_in_logical_order", ARABIC)
        else {
            return;
        };
        let word = "\u{627}\u{628}\u{628}";
        if !covered(&face, word) {
            eprintln!(
                "SKIPPED directwrite_returns_glyphs_in_logical_order: {} lacks Arabic",
                face.family
            );
            return;
        }
        let shaped = shaper.shape(&face, word, EM).expect("shape");

        assert_eq!(shaped.clusters.len(), 3);
        for cluster in &shaped.clusters {
            assert_eq!(
                cluster.glyph_count, 1,
                "every letter must find its own glyph, not an empty range: {:?}",
                shaped.clusters
            );
        }

        let alef = face.glyph_indices(&[0x627])[0];
        assert_eq!(
            shaped.glyphs_of(&shaped.clusters[0]),
            &[alef],
            "alef is the logically first letter and the array puts it first, so the array is in \
             logical order — glyphs were {:?}",
            shaped.glyphs
        );
        assert_eq!(
            shaped
                .clusters
                .iter()
                .map(|c| c.first_glyph)
                .collect::<Vec<_>>(),
            vec![0, 1, 2],
            "a right-to-left run's clusters index into the array in logical order too"
        );
    }

    /// The other half of the finding above: `visual_glyphs` is what puts that array right.
    ///
    /// Same discriminator — alef joins to its right but not its left, so it is identifiable in the
    /// `.cmap` and is unambiguously the *last* thing painted in a right-to-left word. It came back
    /// at glyph index 0 from `GetGlyphs`; it must come back last here.
    #[test]
    fn visual_glyphs_paints_a_right_to_left_run_the_other_way_round() {
        let Some((shaper, face)) = shaper_and_face(
            "visual_glyphs_paints_a_right_to_left_run_the_other_way_round",
            ARABIC,
        ) else {
            return;
        };
        let word = "\u{627}\u{628}\u{628}";
        if !covered(&face, word) {
            eprintln!("SKIPPED visual_glyphs_...: {} lacks Arabic", face.family);
            return;
        }
        let shaped = shaper.shape(&face, word, EM).expect("shape");
        assert!(
            shaped.runs.iter().any(|r| r.level % 2 == 1),
            "the run must actually be marked right-to-left: {:?}",
            shaped.runs
        );

        let order = shaped.visual_glyphs();
        assert_eq!(
            order,
            vec![2, 1, 0],
            "an all-right-to-left word paints back to front; glyphs were {:?}",
            shaped.glyphs
        );

        let alef = face.glyph_indices(&[0x627])[0];
        assert_eq!(
            shaped.glyphs[*order.last().expect("a glyph")],
            alef,
            "alef is logically first and so is painted last (rightmost)"
        );
    }

    /// A Latin line is the overwhelmingly common case and must not be permuted at all.
    #[test]
    fn visual_glyphs_leaves_a_left_to_right_line_alone() {
        let Some((shaper, face)) =
            shaper_and_face("visual_glyphs_leaves_a_left_to_right_line_alone", LATIN)
        else {
            return;
        };
        let shaped = shaper
            .shape(&face, "2026-08-06 WARN retrying", EM)
            .expect("shape");
        assert_eq!(
            shaped.visual_glyphs(),
            (0..shaped.glyphs.len()).collect::<Vec<_>>()
        );
    }

    /// Latin either side of Arabic: the Arabic reverses, the Latin does not, and — the part run-level
    /// reordering alone gets wrong — the glyphs *inside* the Arabic reverse with it.
    #[test]
    fn a_mixed_line_reverses_only_its_right_to_left_run() {
        let Some((shaper, face)) =
            shaper_and_face("a_mixed_line_reverses_only_its_right_to_left_run", ARABIC)
        else {
            return;
        };
        let line = "GET \u{627}\u{628}\u{628} 200";
        if !covered(&face, line) {
            eprintln!("SKIPPED a_mixed_line_...: {} lacks coverage", face.family);
            return;
        }
        let shaped = shaper.shape(&face, line, EM).expect("shape");
        let order = shaped.visual_glyphs();

        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..shaped.glyphs.len()).collect::<Vec<_>>());

        let rtl: Vec<&ShapedRun> = shaped.runs.iter().filter(|r| r.level % 2 == 1).collect();
        assert!(!rtl.is_empty(), "runs were {:?}", shaped.runs);
        for run in rtl {
            let range = run.first_glyph..run.first_glyph + run.glyph_count;
            let painted: Vec<usize> = order
                .iter()
                .copied()
                .filter(|i| range.contains(i))
                .collect();
            let mut descending = painted.clone();
            descending.sort_unstable_by(|a, b| b.cmp(a));
            assert_eq!(
                painted, descending,
                "the glyphs of a right-to-left run must paint in descending logical order; \
                 order was {order:?} for runs {:?}",
                shaped.runs
            );
        }

        for run in shaped.runs.iter().filter(|r| r.level % 2 == 0) {
            let range = run.first_glyph..run.first_glyph + run.glyph_count;
            let painted: Vec<usize> = order
                .iter()
                .copied()
                .filter(|i| range.contains(i))
                .collect();
            let mut ascending = painted.clone();
            ascending.sort_unstable();
            assert_eq!(painted, ascending, "a left-to-right run keeps its order");
        }
    }

    /// A ZWJ emoji sequence is one cluster however many code points it holds — and the shaper must
    /// agree with `cell.rs` about that, or the grid draws six glyphs in a two-column cell.
    #[test]
    fn a_zwj_emoji_sequence_is_one_cluster() {
        let Some((shaper, face)) =
            shaper_and_face("a_zwj_emoji_sequence_is_one_cluster", &["Segoe UI Emoji"])
        else {
            return;
        };
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let shaped = shaper.shape(&face, family, EM).expect("shape");

        assert_eq!(
            shaped.clusters.len(),
            1,
            "five code points and two joiners are one grapheme cluster"
        );
        let span = shaped.clusters[0].span;
        assert_eq!(span.byte, 0);
        assert_eq!(span.byte_len, family.len());
        assert_eq!(
            span.unit_len,
            family.encode_utf16().count(),
            "the UTF-16 half must cover the whole sequence too"
        );
        assert!(
            shaped.clusters[0].glyph_count >= 1,
            "the cluster must draw something"
        );
    }

    /// Latin and Arabic in one line is two script runs and two bidi levels, and the boundaries do
    /// not have to agree. Every cluster must still land in exactly one run.
    #[test]
    fn a_mixed_direction_line_still_accounts_for_every_cluster() {
        let Some((shaper, face)) = shaper_and_face(
            "a_mixed_direction_line_still_accounts_for_every_cluster",
            ARABIC,
        ) else {
            return;
        };
        let line = "user \u{628}\u{62A} logged in";
        let shaped = shaper.shape(&face, line, EM).expect("shape");
        let spans = cluster_spans(line);

        assert_eq!(shaped.clusters.len(), spans.len());
        for (got, want) in shaped.clusters.iter().zip(&spans) {
            assert_eq!(got.span, *want, "clusters must stay in logical order");
        }
        for cluster in &shaped.clusters {
            assert!(
                cluster.first_glyph + cluster.glyph_count <= shaped.glyphs.len(),
                "{cluster:?} runs off the end of {} glyphs",
                shaped.glyphs.len()
            );
        }
    }

    /// An Arabic run really is detected as right-to-left, which is what makes the ordering finding
    /// above a statement about DirectWrite rather than about a run that was never marked.
    #[test]
    fn an_arabic_run_resolves_to_a_right_to_left_bidi_level() {
        let Some((shaper, _face)) = shaper_and_face(
            "an_arabic_run_resolves_to_a_right_to_left_bidi_level",
            ARABIC,
        ) else {
            return;
        };
        let levels = |s: &str| {
            let text: Vec<u16> = s.encode_utf16().collect();
            let spans = cluster_spans(s);
            let line = Line {
                text: &text,
                spans: &spans,
            };
            shaper
                .runs(&line)
                .expect("runs")
                .iter()
                .map(|r| r.right_to_left())
                .collect::<Vec<_>>()
        };

        assert!(
            levels("\u{628}\u{62A}\u{62B}").iter().any(|rtl| *rtl),
            "AnalyzeBidi should resolve an Arabic run to an odd level"
        );
        assert!(
            levels("warn").iter().all(|rtl| !*rtl),
            "and it should leave a Latin run alone"
        );
    }

    /// **The retry loop, driven for real.** Starting from a buffer of one glyph, `GetGlyphs` must
    /// fail with `ERROR_INSUFFICIENT_BUFFER`, be retried at double the size, and eventually return
    /// exactly what a correctly-sized first call returns. If the retry path were broken — the wrong
    /// HRESULT compared, the buffers not re-allocated, `actualGlyphCount` read from the failed call
    /// — this is the only test that would say so.
    #[test]
    fn a_starved_buffer_is_grown_and_gives_the_same_answer() {
        let Some((shaper, face)) =
            shaper_and_face("a_starved_buffer_is_grown_and_gives_the_same_answer", LATIN)
        else {
            return;
        };
        let line = "took 12ms";
        let roomy = shaper.shape(&face, line, EM).expect("shape");
        let starved = shaper
            .shape_from(&face, line, EM, |_| 1)
            .expect("a starved buffer should grow, not fail");

        assert!(!roomy.glyphs.is_empty());
        assert_eq!(starved.glyphs, roomy.glyphs);
        assert_eq!(starved.clusters, roomy.clusters);
        assert_eq!(starved.advances, roomy.advances);
    }

    /// And the growth is bounded. A line too long for `MAX_RETRIES` doublings from one glyph gives
    /// up with an error rather than looping or allocating without limit.
    #[test]
    fn a_buffer_that_never_becomes_big_enough_gives_up() {
        let Some((shaper, face)) =
            shaper_and_face("a_buffer_that_never_becomes_big_enough_gives_up", LATIN)
        else {
            return;
        };
        // 1 glyph doubled MAX_RETRIES times is 16, so 40 characters cannot be reached.
        let line = "x".repeat(40);
        assert!(
            shaper.shape_from(&face, &line, EM, |_| 1).is_err(),
            "the retry loop must be bounded"
        );
        assert!(
            shaper.shape(&face, &line, EM).is_ok(),
            "and the real estimate must be nowhere near that bound"
        );
    }

    /// **No glyph is ever left unclaimed on real text.** Driven over the adversarial alphabet §13.4
    /// cares about — tag characters, variation selectors, bidi controls, zero-width spaces, controls
    /// — which is where a review found `glyph_range` dropping a tag character's glyph on
    /// `\u{200B}\u{FE0F}\u{E0067}`. §5.6 forbids discarding exactly that content silently.
    #[test]
    fn no_glyph_of_a_real_line_goes_unclaimed() {
        let Some((shaper, face)) = shaper_and_face("no_glyph_of_a_real_line_goes_unclaimed", LATIN)
        else {
            return;
        };
        let alphabet = [
            "\u{200B}",
            "\u{FE0F}",
            "\u{FE0E}",
            "\u{E0067}",
            "\u{202E}",
            "\u{200D}",
            "\u{7F}",
            "a",
            "\u{1F600}",
            "\u{301}",
            "\u{FFFD}",
            "\0",
        ];

        let mut lines = 0;
        for a in &alphabet {
            for b in &alphabet {
                for c in &alphabet {
                    let line = format!("{a}{b}{c}");
                    let shaped = shaper.shape(&face, &line, EM).expect("shape");
                    let mut claims = vec![0usize; shaped.glyphs.len()];
                    for cluster in &shaped.clusters {
                        for i in 0..cluster.glyph_count {
                            claims[cluster.first_glyph + i] += 1;
                        }
                    }
                    assert!(
                        claims.iter().all(|c| *c == 1),
                        "{line:?} claimed {claims:?} over {} glyphs, clusters {:?}",
                        shaped.glyphs.len(),
                        shaped.clusters
                    );
                    lines += 1;
                }
            }
        }
        assert!(lines > 1000, "the sweep should be broad, ran {lines}");
    }

    /// **A straddling cluster must not hand its whole line to the wrong script.** `U+093F` is a
    /// spacing mark, so `بि` is one grapheme cluster across the Arabic/Devanagari boundary. When the
    /// snapped boundary collapsed, everything after it shaped as right-to-left Arabic — the conjunct
    /// ligature lost and the cluster 77% wider, which is column drift against §3.3's acceptance test.
    #[test]
    fn a_cluster_straddling_a_script_boundary_does_not_swallow_the_rest_of_the_line() {
        let Some((shaper, face)) = shaper_and_face(
            "a_cluster_straddling_a_script_boundary_does_not_swallow_the_rest_of_the_line",
            DEVANAGARI,
        ) else {
            return;
        };
        let word = "\u{915}\u{94D}\u{937}\u{93F}";
        if !covered(&face, "\u{915}\u{94D}\u{937}\u{93F}") {
            eprintln!(
                "SKIPPED a_cluster_straddling...: {} lacks Devanagari",
                face.family
            );
            return;
        }

        let alone = shaper.shape(&face, word, EM).expect("shape");
        let glyphs_of_word = |prefix: &str| -> Vec<GlyphId> {
            let line = format!("{prefix}{word}");
            let shaped = shaper.shape(&face, &line, EM).expect("shape");
            let from = prefix.len();
            shaped
                .clusters
                .iter()
                .filter(|c| c.span.byte >= from)
                .flat_map(|c| shaped.glyphs_of(c).to_vec())
                .collect()
        };

        // The controls: prefixes that end on a cluster boundary must not disturb the word.
        for prefix in ["x", "\u{628}"] {
            assert_eq!(
                glyphs_of_word(prefix),
                alone.glyphs,
                "the prefix {prefix:?} changed how the Devanagari word shapes"
            );
        }

        // And the case that matters: an Arabic letter followed by a Devanagari **spacing** mark is
        // one grapheme cluster straddling the script boundary. `U+093F` is `Mc`, so it joins the
        // letter before it — which is what puts a cluster across the boundary at all.
        let straddle = "\u{628}\u{93F}";
        assert_eq!(
            cluster_spans(straddle).len(),
            1,
            "the fixture only tests anything if these two really are one cluster"
        );
        assert_eq!(
            glyphs_of_word(straddle),
            alone.glyphs,
            "a cluster straddling the boundary handed the rest of the line to the wrong script"
        );
    }

    #[test]
    fn an_empty_line_shapes_to_nothing() {
        let Some((shaper, face)) = shaper_and_face("an_empty_line_shapes_to_nothing", LATIN) else {
            return;
        };
        let shaped = shaper.shape(&face, "", EM).expect("shape");
        assert!(shaped.clusters.is_empty());
        assert!(shaped.glyphs.is_empty());
    }

    /// The bytes the indexer hands over are decoded text and can contain anything. Shaping must not
    /// panic on any of it, and every reported range must be in bounds.
    #[test]
    fn odd_input_shapes_without_panicking() {
        let Some((shaper, face)) = shaper_and_face("odd_input_shapes_without_panicking", LATIN)
        else {
            return;
        };
        for line in [
            "\u{FFFD}\u{FFFD}",
            "\u{202E}abc",
            "\u{200B}\u{200B}",
            "\r\n",
            "\0\u{1}\u{7F}",
            "a\u{E0067}b",
            "\u{1F3FB}",
            "\u{FE0F}",
        ] {
            let shaped = shaper.shape(&face, line, EM).expect("shape");
            assert_eq!(shaped.clusters.len(), cluster_spans(line).len(), "{line:?}");
            for cluster in &shaped.clusters {
                assert!(
                    cluster.first_glyph + cluster.glyph_count <= shaped.glyphs.len(),
                    "{line:?}: {cluster:?} escapes {} glyphs",
                    shaped.glyphs.len()
                );
                assert_eq!(
                    &line[cluster.span.byte..cluster.span.byte + cluster.span.byte_len]
                        .encode_utf16()
                        .count(),
                    &cluster.span.unit_len,
                    "{line:?}: the two halves of {cluster:?} describe different text"
                );
            }
        }
    }
}
