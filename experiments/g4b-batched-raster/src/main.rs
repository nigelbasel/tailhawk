//! G4b — batched glyph rasterisation.
//!
//! Follow-up to the open caveat in `experiments/g4-glyph-atlas/RESULTS.md`: *"Rasterisation cost is
//! measured at one-glyph-per-analysis granularity. Batched runs are untested."* G4 measured
//! DirectWrite at 145–390 µs per glyph using one `CreateGlyphRunAnalysis` per glyph, which makes a
//! cold viewport of ~1,500 CJK glyphs cost 14–35 frames and is the renderer's dominant expense.
//!
//! Three questions, in this order, because each one can make the next meaningless:
//!
//! 1. **What does a glyph's alpha-texture bound actually measure?** The atlas cell has to be
//!    derived from that, not guessed — a cell that clips is a cell whose contents compare equal for
//!    the wrong reason.
//! 2. **Does DirectWrite cache rasterised glyphs internally?** If it does, every figure here has to
//!    be taken cold, and G4's number needs re-reading too.
//! 3. **Is a glyph rasterised inside a run bit-identical to the same glyph rasterised alone, and is
//!    it faster?** Identical first: if batching perturbs the raster it cannot feed an atlas at any
//!    speed, because a cell would depend on what happened to sit next to it.
//!
//! Both arms produce the *same artefact* — one uniform atlas cell per glyph — so the batched arm
//! pays for slicing its wide bitmap back into cells. Timing the analysis alone would be a rigged
//! comparison: the atlas needs cells, not a strip.
//!
//! No window, no D3D device, no swapchain. That is deliberate — it makes this experiment immune to
//! the leaked-subject trap that invalidated a round of G3 conclusions (see `docs/HANDOFF.md`).
//!
//! What it found, which is not what it set out to find: batching is bit-identical and worth ~1.8x
//! saturating at 4 glyphs per analysis, but per-analysis overhead was never the dominant cost. A
//! **cross-process, capacity-limited system font cache** is — first process on the machine pays
//! ~86–108 µs/glyph, every later process ~3 µs, capacity 8,000–16,000 distinct glyphs. G4's figure
//! is a thrash figure from a fixture that cycles 20,992 glyphs. See `RESULTS.md`.
//!
//! Modes:
//!   `g4b-batched-raster`             full report: bounds, cache probe, correctness, warm sweep
//!   `g4b-batched-raster cold <n>`    one cold pass at batch size <n>, one CSV line, then exit —
//!                                    the only mode that measures rasterisation rather than cache
//!                                    hits, and only on its first run per (glyph, size) population
//!   `g4b-batched-raster coldsweep`   batch-size sweep on disjoint cold slices, both orders
//!   `g4b-batched-raster capacity`    where the font cache stops holding the set, by cold/warm ratio
//!   `g4b-batched-raster g4set`       replicates G4's 20,992-glyph cycling fixture, no D3D

mod fonts;

use std::time::Instant;

use windows::core::Result;
use windows::Win32::Foundation::{BOOL, RECT};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFontFace, DWRITE_GLYPH_RUN, DWRITE_GRID_FIT_MODE_DEFAULT, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, DWRITE_TEXTURE_CLEARTYPE_3x1,
    DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
};

/// Matches the cold-viewport figure G4 designed against: ~1,500 previously-unseen glyphs.
const N_GLYPHS: usize = 1500;
const EM: f32 = 14.0;
const TRIALS: usize = 7;
const BATCHES: [usize; 7] = [1, 4, 16, 64, 256, 1024, N_GLYPHS];
/// Gap between adjacent cells in a batched run, in device pixels. Swept in the correctness section;
/// this is the value the timing arms use.
const PAD: i32 = 4;
const PADS: [i32; 4] = [0, 2, 4, 8];

/// Uniform atlas cell, in device pixels relative to a baseline origin at (0, 0). Derived from
/// measured glyph bounds rather than assumed — see `Cell::measure`.
#[derive(Clone, Copy)]
struct Cell {
    /// Left edge relative to the pen. Negative for a negative left side bearing — the cell has to
    /// be offset to hold that ink, not merely widened.
    left: i32,
    w: i32,
    h: i32,
    top: i32,
}

impl Cell {
    fn bytes(&self) -> usize {
        (self.w * self.h * 3) as usize
    }
}

#[derive(Default, Clone, Copy)]
struct Timing {
    create: f64,
    bounds: f64,
    texture: f64,
    copy: f64,
}

impl Timing {
    fn total(&self) -> f64 {
        self.create + self.bounds + self.texture + self.copy
    }
}

struct Arm {
    timing: Timing,
    cells: Vec<u8>,
    /// Glyphs whose ink fell outside the uniform cell. Must be 0 for a comparison to mean anything.
    overflow: usize,
    /// Glyphs with no ink at all (absent from the face).
    blank: usize,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_nanos() as f64 / 1_000_000.0
}

fn make_run(face: &IDWriteFontFace, glyphs: &[u16], advances: &[f32], em: f32) -> DWRITE_GLYPH_RUN {
    DWRITE_GLYPH_RUN {
        fontFace: unsafe { std::mem::transmute_copy(face) },
        fontEmSize: em,
        glyphCount: glyphs.len() as u32,
        glyphIndices: glyphs.as_ptr(),
        glyphAdvances: advances.as_ptr(),
        glyphOffsets: std::ptr::null(),
        isSideways: BOOL(0),
        bidiLevel: 0,
    }
}

/// Per-glyph alpha-texture bounds, one analysis per glyph. Used to derive the cell geometry and to
/// report what the bounds actually are, rather than assuming em size bounds the ink.
fn per_glyph_bounds(f: &fonts::Fonts, face: &IDWriteFontFace, glyphs: &[u16]) -> Result<Vec<RECT>> {
    let advance = [0.0f32];
    let mut out = Vec::with_capacity(glyphs.len());
    for g in glyphs {
        let one = [*g];
        let run = make_run(face, &one, &advance, EM);
        let analysis = unsafe {
            f.factory.CreateGlyphRunAnalysis(
                &run,
                None,
                DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                0.0,
                0.0,
            )?
        };
        out.push(unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)? });
    }
    Ok(out)
}

impl Cell {
    /// The smallest uniform cell that contains every glyph's ink, with the advance rounded up so
    /// adjacent glyphs in a batched run cannot overlap.
    fn measure(bounds: &[RECT]) -> Self {
        let ink: Vec<&RECT> = bounds
            .iter()
            .filter(|b| b.right > b.left && b.bottom > b.top)
            .collect();
        let left = ink.iter().map(|b| b.left).min().unwrap_or(0);
        let right = ink.iter().map(|b| b.right).max().unwrap_or(0);
        let top = ink.iter().map(|b| b.top).min().unwrap_or(0);
        let bottom = ink.iter().map(|b| b.bottom).max().unwrap_or(0);
        Self {
            left,
            w: right - left,
            h: bottom - top,
            top,
        }
    }
}

/// Rasterise `glyphs` in chunks of `batch`, writing one uniform cell per glyph.
///
/// `batch == 1` reproduces G4's granularity exactly and doubles as the correctness reference.
fn rasterise(
    f: &fonts::Fonts,
    face: &IDWriteFontFace,
    glyphs: &[u16],
    batch: usize,
    cell: Cell,
    pad: i32,
    em: f32,
) -> Result<Arm> {
    let mut cells = vec![0u8; glyphs.len() * cell.bytes()];
    let mut t = Timing::default();
    let (mut overflow, mut blank) = (0usize, 0usize);

    // Hoisted so allocation is not charged to whichever arm allocates more often. A real renderer
    // reuses a scratch buffer; charging 1,500 small allocations to the per-glyph arm would inflate
    // the batched win with something that is not a property of batching.
    let mut scratch: Vec<u8> = Vec::new();
    let step = cell.w + pad;
    let advances = vec![step as f32; batch.min(glyphs.len())];

    for (chunk_index, chunk) in glyphs.chunks(batch).enumerate() {
        let base = chunk_index * batch;
        let run = make_run(face, chunk, &advances, em);

        let t0 = Instant::now();
        let analysis = unsafe {
            f.factory.CreateGlyphRunAnalysis(
                &run,
                None,
                DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                0.0,
                0.0,
            )?
        };
        t.create += ms(t0);

        let t1 = Instant::now();
        let b: RECT = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)? };
        t.bounds += ms(t1);

        let (w, h) = (b.right - b.left, b.bottom - b.top);
        if w <= 0 || h <= 0 {
            blank += chunk.len();
            continue;
        }

        scratch.clear();
        scratch.resize((w * h * 3) as usize, 0);
        let t2 = Instant::now();
        unsafe { analysis.CreateAlphaTexture(DWRITE_TEXTURE_CLEARTYPE_3x1, &b, &mut scratch)? };
        t.texture += ms(t2);

        // With one glyph per analysis the run bounds *are* the glyph's bounds, so this is the only
        // arm that can check containment exactly.
        if batch == 1
            && (b.left < cell.left
                || b.right > cell.left + cell.w
                || b.top < cell.top
                || b.bottom > cell.top + cell.h)
        {
            overflow += 1;
        }

        let t3 = Instant::now();
        for i in 0..chunk.len() {
            let pen_x = i as i32 * step + cell.left;
            let dst_base = (base + i) * cell.bytes();
            for y in 0..cell.h {
                let ay = cell.top + y;
                if ay < b.top || ay >= b.bottom {
                    continue;
                }
                let src_row = ((ay - b.top) * w) as usize;
                for x in 0..cell.w {
                    let ax = pen_x + x;
                    if ax < b.left || ax >= b.right {
                        continue;
                    }
                    let s = (src_row + (ax - b.left) as usize) * 3;
                    let d = dst_base + ((y * cell.w + x) * 3) as usize;
                    cells[d..d + 3].copy_from_slice(&scratch[s..s + 3]);
                }
            }
        }
        t.copy += ms(t3);
    }

    Ok(Arm {
        timing: t,
        cells,
        overflow,
        blank,
    })
}

/// A byte-for-byte replica of `g4-glyph-atlas`'s `text::raster_mono`, including the three things
/// this experiment's own arm deliberately does not do: clone the face per glyph (a COM AddRef),
/// allocate two fresh `Vec`s per glyph, and convert ClearType coverage to RGBA.
///
/// Its only purpose is reconciliation. G4 reported 330–388 µs per glyph for this exact path on a
/// quiet machine; if that reproduces here, the cost is in one of those three additions, and if it
/// does not, the cost was never in `raster_mono` at all.
fn rasterise_g4_shape(
    f: &fonts::Fonts,
    face: &IDWriteFontFace,
    glyphs: &[u16],
    em: f32,
) -> Result<f64> {
    let advance = [0.0f32];
    let t = Instant::now();
    let mut sink = 0u64;
    for g in glyphs {
        let face = face.clone();
        let one = [*g];
        let run = make_run(&face, &one, &advance, em);
        let analysis = unsafe {
            f.factory.CreateGlyphRunAnalysis(
                &run,
                None,
                DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC,
                DWRITE_MEASURING_MODE_NATURAL,
                DWRITE_GRID_FIT_MODE_DEFAULT,
                DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
                0.0,
                0.0,
            )?
        };
        let b = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)? };
        let (w, h) = (b.right - b.left, b.bottom - b.top);
        if w <= 0 || h <= 0 {
            continue;
        }
        let mut cov = vec![0u8; (w * h * 3) as usize];
        unsafe { analysis.CreateAlphaTexture(DWRITE_TEXTURE_CLEARTYPE_3x1, &b, &mut cov)? };
        let (w, h) = (w as usize, h as usize);
        let mut pixels = vec![0u8; w * h * 4];
        for i in 0..w * h {
            let (r, g, b) = (cov[i * 3], cov[i * 3 + 1], cov[i * 3 + 2]);
            let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
            pixels[i * 4] = r;
            pixels[i * 4 + 1] = g;
            pixels[i * 4 + 2] = b;
            pixels[i * 4 + 3] = avg;
        }
        // Keep the buffer observably live so nothing above can be optimised away.
        sink = sink.wrapping_add(pixels[pixels.len() - 1] as u64);
    }
    std::hint::black_box(sink);
    Ok(ms(t))
}

/// Number of glyph cells that differ, and the first differing index.
fn compare(reference: &[u8], other: &[u8], cell: Cell) -> (usize, Option<usize>) {
    let n = cell.bytes();
    let mut differing = 0usize;
    let mut first = None;
    for i in 0..reference.len() / n {
        if reference[i * n..(i + 1) * n] != other[i * n..(i + 1) * n] {
            differing += 1;
            first.get_or_insert(i);
        }
    }
    (differing, first)
}

fn p50(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    v[(v.len() - 1) / 2]
}

/// The glyph set: a contiguous CJK block gives 1,500 distinct glyphs with no duplicates, which is
/// what a cold viewport of unseen text looks like to the atlas.
fn glyph_set(f: &fonts::Fonts) -> Vec<u16> {
    let codepoints: Vec<u32> = (0..N_GLYPHS as u32).map(|i| 0x4E00 + i).collect();
    f.glyph_indices(&codepoints)
}

/// One cold pass, one CSV line, then exit. Driven across processes by `cold.ps1` so that every
/// measured pass is the first rasterisation of those glyphs in that process.
fn cold(batch: usize) -> Result<()> {
    let f = fonts::Fonts::new(&["Microsoft YaHei", "MS Gothic", "SimSun", "Malgun Gothic"])?;
    // Nothing may touch DirectWrite before the timed pass. Deriving the cell from measured bounds
    // would rasterise every glyph first; the cell is therefore hardcoded to what the full report
    // measured (16x15 at (-1, -12) for Microsoft YaHei at em 14). `overflow` still verifies it.
    let glyphs = glyph_set(&f);
    let cell = Cell {
        left: -1,
        w: 16,
        h: 15,
        top: -12,
    };
    let arm = rasterise(&f, &f.face, &glyphs, batch, cell, PAD, EM)?;
    let t = arm.timing;
    let line = format!(
        "{batch},{:.3},{:.3},{:.3},{:.3},{:.3},{:.2},{},{}",
        t.total(),
        t.create,
        t.bounds,
        t.texture,
        t.copy,
        t.total() * 1000.0 / N_GLYPHS as f64,
        arm.overflow,
        arm.blank
    );
    println!("{line}");
    let _ = std::fs::write(
        std::env::temp_dir().join("g4b-batched-raster-cold.txt"),
        &line,
    );
    Ok(())
}

/// Replicate `g4-glyph-atlas`'s CJK overflow phase exactly — the same 20,992-codepoint ideograph
/// block, the same 1,500-glyph frames cycling through it, the same 120 frames, the same call shape
/// — but with no D3D device, no swapchain, no atlas and no uploads.
///
/// This isolates the one variable left between G4's 92–97 µs/glyph and this experiment's 4.0: the
/// glyph set. G4 draws from the whole block; the main report draws the first 1,500 codepoints.
fn g4set() -> Result<()> {
    const BASE: u32 = 0x4E00;
    const SPAN: u32 = 20_992;
    const FRAMES: usize = 120;
    const PER_FRAME: usize = 1500;

    let f = fonts::Fonts::new(&["Microsoft YaHei", "MS Gothic", "SimSun", "Malgun Gothic"])?;
    let all: Vec<u32> = (0..SPAN).map(|i| BASE + i).collect();
    let block = f.glyph_indices(&all);
    let notdef = block.iter().filter(|&&g| g == 0).count();
    let distinct = {
        let mut g = block.clone();
        g.sort_unstable();
        g.dedup();
        g.len()
    };
    println!("block {SPAN} codepoints | {distinct} distinct glyphs | {notdef} notdef");

    // The whole block once, so a cost that varies across the block shows up as a spread.
    let quarter = SPAN as usize / 4;
    for q in 0..4 {
        let slice = &block[q * quarter..(q + 1) * quarter];
        let t = rasterise_g4_shape(&f, &f.face, slice, EM)?;
        println!(
            "block quarter {q} (U+{:04X}..): {t:8.2} ms for {} glyphs = {:.1} us/glyph",
            BASE + (q * quarter) as u32,
            slice.len(),
            t * 1000.0 / slice.len() as f64
        );
    }

    // G4's frame loop, verbatim in shape.
    let mut per_frame = Vec::new();
    for frame in 0..FRAMES {
        let idx: Vec<u16> = (0..PER_FRAME)
            .map(|i| block[(frame * PER_FRAME + i) % SPAN as usize])
            .collect();
        per_frame.push(rasterise_g4_shape(&f, &f.face, &idx, EM)?);
    }
    let total: f64 = per_frame.iter().sum();
    let mut sorted = per_frame.clone();
    println!(
        "\n{FRAMES} frames x {PER_FRAME} glyphs: total {:.1} s | per frame p50 {:.2} ms | \
         first {:.2} | last {:.2} | {:.1} us/glyph",
        total / 1000.0,
        p50(&mut sorted),
        per_frame[0],
        per_frame[FRAMES - 1],
        total * 1000.0 / (FRAMES * PER_FRAME) as f64
    );
    println!("G4 on the same machine reports 138.67 / 144.89 ms per frame for this phase.");
    Ok(())
}

/// Find where the shared font cache runs out.
///
/// The `cold` mode showed the first process to rasterise a glyph pays ~108 µs and every later
/// process pays ~3 µs, so the cache outlives the process — it is the system font cache, not
/// per-process state. The number that matters to the renderer is therefore not "how fast is
/// rasterisation" but "how many distinct glyphs stay resident".
///
/// Each size gets its own em, because the cache key includes size: that makes every measurement
/// cold regardless of what earlier runs touched, without needing a reboot between them. Per-glyph
/// cost rises with em, so the signal is the cold/warm *ratio*, which is scale-free.
fn capacity() -> Result<()> {
    let f = fonts::Fonts::new(&["Microsoft YaHei", "MS Gothic", "SimSun", "Malgun Gothic"])?;
    let all: Vec<u32> = (0..20_992u32).map(|i| 0x4E00 + i).collect();
    let block = f.glyph_indices(&all);

    println!(
        "{:>7} {:>5} {:>11} {:>11} {:>11} {:>8}",
        "glyphs", "em", "cold us/g", "warm us/g", "warm2 us/g", "cold/warm"
    );
    println!("{}", "-".repeat(60));
    for (i, &s) in [500usize, 1000, 2000, 4000, 8000, 16000].iter().enumerate() {
        let em = 20.0 + i as f32;
        let g = &block[..s];
        let cold = rasterise_g4_shape(&f, &f.face, g, em)? * 1000.0 / s as f64;
        let warm = rasterise_g4_shape(&f, &f.face, g, em)? * 1000.0 / s as f64;
        let warm2 = rasterise_g4_shape(&f, &f.face, g, em)? * 1000.0 / s as f64;
        println!(
            "{s:>7} {em:>5.0} {cold:>11.1} {warm:>11.1} {warm2:>11.1} {:>7.1}x",
            cold / warm
        );
    }
    println!(
        "\nA ratio near 1 means the set no longer fits: the second pass is still paying to \
         rasterise."
    );
    Ok(())
}

/// The batch-size sweep under *cold* rasterisation.
///
/// The warm sweep in the full report measures cache lookups, not rasterisation, so it cannot settle
/// whether batching helps when the work is real — which is the question the experiment exists to
/// answer. Every arm therefore gets a disjoint slice of the ideograph block, so no arm can benefit
/// from a previous arm's misses, and the whole sweep is repeated at a second em in reverse order to
/// catch any warm-up trend across arms.
fn coldsweep() -> Result<()> {
    const SLICE: usize = 1000;
    let f = fonts::Fonts::new(&["Microsoft YaHei", "MS Gothic", "SimSun", "Malgun Gothic"])?;
    let all: Vec<u32> = (0..20_992u32).map(|i| 0x4E00 + i).collect();
    let block = f.glyph_indices(&all);

    println!("{SLICE} cold glyphs per arm, disjoint slices, no arm reuses another's glyphs");
    println!(
        "{:>7} {:>16} {:>16} {:>10}",
        "batch", "em 17 fwd us/g", "em 18 rev us/g", "vs b1"
    );
    println!("{}", "-".repeat(54));

    let mut result = [[0.0f64; 2]; BATCHES.len()];
    for (col, (em, reverse)) in [(17.0f32, false), (18.0f32, true)].iter().enumerate() {
        // Sized for em 18: em-14 ink was <=16x14 at left -1, top -12, so 1.3x with margin.
        let cell = Cell {
            left: -2,
            w: 24,
            h: 21,
            top: -17,
        };
        for step in 0..BATCHES.len() {
            let k = if *reverse {
                BATCHES.len() - 1 - step
            } else {
                step
            };
            let slice = &block[k * SLICE..(k + 1) * SLICE];
            let arm = rasterise(&f, &f.face, slice, BATCHES[k].min(SLICE), cell, PAD, *em)?;
            if arm.overflow != 0 {
                println!("  !! batch {} overflowed {} cells", BATCHES[k], arm.overflow);
            }
            result[k][col] = arm.timing.total() * 1000.0 / SLICE as f64;
        }
    }
    for (k, &b) in BATCHES.iter().enumerate() {
        let mean = (result[k][0] + result[k][1]) / 2.0;
        let base = (result[0][0] + result[0][1]) / 2.0;
        println!(
            "{:>7} {:>16.1} {:>16.1} {:>9.2}x",
            b.min(SLICE),
            result[k][0],
            result[k][1],
            base / mean
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "capacity" {
        return capacity();
    }
    if args.len() >= 2 && args[1] == "coldsweep" {
        return coldsweep();
    }
    if args.len() >= 3 && args[1] == "cold" {
        return cold(args[2].parse().expect("batch size"));
    }
    if args.len() >= 2 && args[1] == "g4set" {
        return g4set();
    }

    let mut out = String::new();
    macro_rules! say {
        ($($a:tt)*) => {{
            let line = format!($($a)*);
            println!("{line}");
            out.push_str(&line);
            out.push('\n');
        }};
    }

    let f = fonts::Fonts::new(&["Microsoft YaHei", "MS Gothic", "SimSun", "Malgun Gothic"])?;
    let glyphs = glyph_set(&f);
    let distinct = {
        let mut g = glyphs.clone();
        g.sort_unstable();
        g.dedup();
        g.len()
    };

    say!("G4b — batched glyph rasterisation");
    say!(
        "face {} | em {EM} | {N_GLYPHS} glyphs ({distinct} distinct)",
        f.name
    );
    say!("");

    // ---- Question 1: what are the bounds, and what cell do they imply? ----
    // This is also the first rasterisation of these glyphs in the process, so it is timed: it is
    // the cold figure that everything after it is compared against.
    say!("== bounds: what one glyph's alpha texture actually spans ==");
    let t_first = Instant::now();
    let bounds = per_glyph_bounds(&f, &f.face, &glyphs)?;
    let first_pass = ms(t_first);
    let cell = Cell::measure(&bounds);
    let widths: Vec<i32> = bounds.iter().map(|b| b.right - b.left).collect();
    let heights: Vec<i32> = bounds.iter().map(|b| b.bottom - b.top).collect();
    say!(
        "left {}..{} | right {}..{} | top {}..{} | bottom {}..{}",
        bounds.iter().map(|b| b.left).min().unwrap_or(0),
        bounds.iter().map(|b| b.left).max().unwrap_or(0),
        bounds.iter().map(|b| b.right).min().unwrap_or(0),
        bounds.iter().map(|b| b.right).max().unwrap_or(0),
        bounds.iter().map(|b| b.top).min().unwrap_or(0),
        bounds.iter().map(|b| b.top).max().unwrap_or(0),
        bounds.iter().map(|b| b.bottom).min().unwrap_or(0),
        bounds.iter().map(|b| b.bottom).max().unwrap_or(0),
    );
    say!(
        "ink w {}..{} | ink h {}..{} => derived cell {}x{} at ({}, {})",
        widths.iter().min().unwrap_or(&0),
        widths.iter().max().unwrap_or(&0),
        heights.iter().min().unwrap_or(&0),
        heights.iter().max().unwrap_or(&0),
        cell.w,
        cell.h,
        cell.left,
        cell.top
    );
    say!(
        "first rasterisation of all {N_GLYPHS} glyphs (bounds only, no texture): {first_pass:.2} ms"
    );
    say!("");

    // ---- Question 2: is there an internal cache? ----
    // Five successive identical passes. If pass 1 dominates, every later figure is a cache hit and
    // has to be taken cold instead. G4's ~18% ordering effect was read as evidence against a cache;
    // that reading is only safe if this probe agrees.
    say!("== cache probe: five identical passes, batch 1 ==");
    let mut probe = Vec::new();
    for pass in 0..5 {
        let arm = rasterise(&f, &f.face, &glyphs, 1, cell, PAD, EM)?;
        probe.push(arm.timing.total());
        say!(
            "pass {pass}: {:.2} ms ({:.1} us/glyph) | overflow {} | blank {}",
            arm.timing.total(),
            arm.timing.total() * 1000.0 / N_GLYPHS as f64,
            arm.overflow,
            arm.blank
        );
    }
    // ⚠ This probe cannot see the cache that matters, and saying so is the point. The bounds
    // section above already rasterised every glyph, and the cache is shared across processes, so
    // pass 0 is warm before it starts. A flat result here means nothing; only `cold`, run as the
    // first DirectWrite work in a fresh process, measures a miss. Kept because reading it as
    // "no cache" is the exact mistake that made the first version of this experiment report 2.5
    // us/glyph and conclude G4 was wrong by 150x.
    say!(
        "pass0 / pass4 = {:.2}x — flat, but this probe is warm on entry and cannot detect the \
         cross-process cache. See `cold`.",
        probe[0] / probe[4]
    );
    say!("");

    // ---- Reconciliation: does G4's own call shape reproduce G4's number? ----
    say!("== reconciliation: this arm vs a replica of G4's raster_mono ==");
    let mine = rasterise(&f, &f.face, &glyphs, 1, cell, PAD, EM)?.timing.total();
    let g4 = rasterise_g4_shape(&f, &f.face, &glyphs, EM)?;
    say!(
        "batch 1, hoisted scratch : {mine:>8.2} ms ({:.1} us/glyph)",
        mine * 1000.0 / N_GLYPHS as f64
    );
    say!(
        "g4 raster_mono replica   : {g4:>8.2} ms ({:.1} us/glyph)",
        g4 * 1000.0 / N_GLYPHS as f64
    );
    say!(
        "G4 reported 330-388 us/glyph for this path on a quiet machine; the replica is {:.0}x {} that.",
        (330.0 / (g4 * 1000.0 / N_GLYPHS as f64)).max(g4 * 1000.0 / N_GLYPHS as f64 / 330.0),
        if g4 * 1000.0 / N_GLYPHS as f64 > 330.0 { "above" } else { "below" }
    );
    say!("");

    // ---- Question 3a: does batching perturb the raster? ----
    // Swept over the inter-cell gap, because the ClearType filter is horizontal: if a run needs
    // padding to stay identical, that is a constraint on the batching scheme, not a failure of it.
    say!("== correctness: batched cells vs per-glyph cells, by inter-cell pad ==");
    let mut usable_pad = None;
    for &pad in PADS.iter() {
        let reference = rasterise(&f, &f.face, &glyphs, 1, cell, pad, EM)?;
        let mut worst = 0usize;
        let mut detail = String::new();
        for &b in BATCHES.iter().skip(1) {
            let arm = rasterise(&f, &f.face, &glyphs, b, cell, pad, EM)?;
            let (differing, _) = compare(&reference.cells, &arm.cells, cell);
            worst = worst.max(differing);
            detail.push_str(&format!(" b{b}={differing}"));
        }
        say!(
            "pad {pad:>2}: overflow {} | worst {worst:>5} of {N_GLYPHS} differ |{detail}",
            reference.overflow
        );
        if worst == 0 && reference.overflow == 0 && usable_pad.is_none() {
            usable_pad = Some(pad);
        }
    }
    say!(
        "verdict: {}",
        match usable_pad {
            Some(p) =>
                format!("USABLE — bit-identical to per-glyph rasterisation at an inter-cell pad of {p} px"),
            None => "NOT USABLE at any pad tried — a glyph in a run differs from the same glyph alone"
                .to_string(),
        }
    );
    say!("");

    // ---- Question 3b: how much faster, and where does it saturate? ----
    // Rotating start order per trial: G4 found an ~18% within-process ordering effect between two
    // phases doing identical work, so a fixed order would confound the batch-size comparison.
    say!("== warm timing: {TRIALS} trials, rotating order ==");
    let mut totals: Vec<Vec<f64>> = vec![Vec::new(); BATCHES.len()];
    let mut breakdown: Vec<Vec<Timing>> = vec![Vec::new(); BATCHES.len()];
    for trial in 0..TRIALS {
        let mut line = format!("trial {trial}:");
        for step in 0..BATCHES.len() {
            let k = (trial + step) % BATCHES.len();
            let arm = rasterise(&f, &f.face, &glyphs, BATCHES[k], cell, PAD, EM)?;
            totals[k].push(arm.timing.total());
            breakdown[k].push(arm.timing);
            line.push_str(&format!(" b{}={:.1}", BATCHES[k], arm.timing.total()));
        }
        say!("{line}");
    }
    say!("");

    say!(
        "{:>7} {:>9} {:>9} {:>9} {:>10} {:>8} {:>8} {:>8} {:>9} {:>8}",
        "batch",
        "p50 ms",
        "min ms",
        "max ms",
        "us/glyph",
        "vs b1",
        "create",
        "bounds",
        "texture",
        "copy"
    );
    say!("{}", "-".repeat(97));
    let base = p50(&mut totals[0].clone());
    for (k, &b) in BATCHES.iter().enumerate() {
        let mut v = totals[k].clone();
        let med = p50(&mut v);
        let n = breakdown[k].len() as f64;
        let avg = |sel: fn(&Timing) -> f64| breakdown[k].iter().map(sel).sum::<f64>() / n;
        say!(
            "{b:>7} {med:>9.2} {:>9.2} {:>9.2} {:>10.1} {:>7.2}x {:>8.2} {:>8.2} {:>9.2} {:>8.2}",
            v[0],
            v[v.len() - 1],
            med * 1000.0 / N_GLYPHS as f64,
            base / med,
            avg(|t| t.create),
            avg(|t| t.bounds),
            avg(|t| t.texture),
            avg(|t| t.copy),
        );
    }
    say!("");

    // Paired win counts: the only comparison immune to load drift between trials.
    say!("== paired: each batch size against batch 1, same trial ==");
    for (k, &b) in BATCHES.iter().enumerate().skip(1) {
        let wins = totals[k]
            .iter()
            .zip(&totals[0])
            .filter(|(x, y)| x < y)
            .count();
        let ratios: Vec<f64> = totals[k].iter().zip(&totals[0]).map(|(x, y)| y / x).collect();
        let mut r = ratios.clone();
        say!(
            "batch {b:>5}: wins {wins}/{TRIALS} | speedup p50 {:.2}x | min {:.2}x | max {:.2}x",
            p50(&mut r),
            ratios.iter().cloned().fold(f64::INFINITY, f64::min),
            ratios.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        );
    }

    let path = std::env::temp_dir().join("g4b-batched-raster.txt");
    let _ = std::fs::write(&path, &out);
    println!("\nreport written to {}", path.display());
    Ok(())
}
