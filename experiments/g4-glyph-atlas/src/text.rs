//! DirectWrite rasterisation: a monochrome alpha texture via `IDWriteGlyphRunAnalysis`, and a
//! premultiplied colour bitmap via `IDWriteFactory2::TranslateColorGlyphRun`.

use windows::core::{Result, HRESULT, PCWSTR};
use windows::Win32::Foundation::{BOOL, RECT};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory2, IDWriteFontFace, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_GLYPH_RUN,
    DWRITE_GRID_FIT_MODE_DEFAULT, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_RENDERING_MODE_NATURAL_SYMMETRIC, DWRITE_TEXTURE_CLEARTYPE_3x1,
    DWRITE_TEXT_ANTIALIAS_MODE_CLEARTYPE,
};

/// `DWRITE_E_NOCOLOR` — the run has no colour layers. Not an error, just a monochrome glyph.
const DWRITE_E_NOCOLOR: HRESULT = HRESULT(0x8898_500Cu32 as i32);

/// Index into `Fonts::faces`, and the `face` field of an `atlas::GlyphKey`.
pub const FACE_MONO: u16 = 0;
pub const FACE_CJK: u16 = 1;
pub const FACE_EMOJI: u16 = 2;

pub struct Fonts {
    pub factory: IDWriteFactory2,
    pub faces: Vec<IDWriteFontFace>,
    pub names: Vec<String>,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Fonts {
    /// Resolves each requested family by name. Font *fallback* is deliberately not used — G4 is
    /// about atlas composition, and hand-picking one face per script keeps the glyph-run
    /// construction direct. Mixed-script fallback is a separate concern (`SPEC.md` §6).
    pub fn new() -> Result<Self> {
        let factory: IDWriteFactory2 =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let mut collection = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false)? };
        let collection = collection.expect("system font collection");

        // Candidate lists, first hit wins. Ordered by how likely they are to be present.
        let wanted: [&[&str]; 3] = [
            &["Cascadia Mono", "Consolas", "Courier New"],
            &["Microsoft YaHei", "MS Gothic", "SimSun", "Malgun Gothic"],
            &["Segoe UI Emoji"],
        ];

        let mut faces = Vec::new();
        let mut names = Vec::new();
        for candidates in wanted {
            let mut chosen = None;
            for name in candidates {
                let w = wide(name);
                let mut index = 0u32;
                let mut exists = BOOL(0);
                unsafe {
                    collection.FindFamilyName(PCWSTR(w.as_ptr()), &mut index, &mut exists)?;
                }
                if !exists.as_bool() {
                    continue;
                }
                let family = unsafe { collection.GetFontFamily(index)? };
                let font = unsafe {
                    family.GetFirstMatchingFont(
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                    )?
                };
                let face = unsafe { font.CreateFontFace()? };
                chosen = Some((face, (*name).to_string()));
                break;
            }
            match chosen {
                Some((face, name)) => {
                    faces.push(face);
                    names.push(name);
                }
                None => {
                    // Fall back to the first resolved face so indices stay stable; the report
                    // names what was actually used.
                    let first = faces
                        .first()
                        .expect("first candidate list must resolve")
                        .clone();
                    faces.push(first);
                    names.push(format!("MISSING -> {}", names[0]));
                }
            }
        }

        Ok(Self {
            factory,
            faces,
            names,
        })
    }

    pub fn face(&self, idx: u16) -> &IDWriteFontFace {
        &self.faces[idx as usize]
    }

    pub fn glyph_index(&self, idx: u16, codepoint: u32) -> u16 {
        let mut out = [0u16; 1];
        unsafe {
            let _ = self
                .face(idx)
                .GetGlyphIndices(&codepoint, 1, out.as_mut_ptr());
        }
        out[0]
    }
}

/// A rasterised glyph box: `bounds` in device pixels relative to the pen origin, and pixels in the
/// atlas's own format.
pub struct Raster {
    pub bounds: RECT,
    /// RGBA8. For mono: RGB are the three ClearType subpixel coverages and A is their average, so
    /// one sheet serves both subpixel and greyscale rendering. For colour: premultiplied BGRA.
    pub pixels: Vec<u8>,
    pub colour: bool,
}

impl Raster {
    pub fn width(&self) -> u32 {
        (self.bounds.right - self.bounds.left).max(0) as u32
    }
    pub fn height(&self) -> u32 {
        (self.bounds.bottom - self.bounds.top).max(0) as u32
    }
    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }
}

fn make_run(face: &IDWriteFontFace, glyph: &u16, em: f32, advance: &f32) -> DWRITE_GLYPH_RUN {
    DWRITE_GLYPH_RUN {
        fontFace: unsafe { std::mem::transmute_copy(face) },
        fontEmSize: em,
        glyphCount: 1,
        glyphIndices: glyph,
        glyphAdvances: advance,
        glyphOffsets: std::ptr::null(),
        isSideways: BOOL(0),
        bidiLevel: 0,
    }
}

/// ClearType 3x1 coverage for one glyph. Returns `None` for a glyph with no ink (e.g. space).
fn raster_layer(
    fonts: &Fonts,
    face: &IDWriteFontFace,
    glyph: u16,
    em: f32,
) -> Result<Option<(RECT, Vec<u8>)>> {
    let advance = 0.0f32;
    let run = make_run(face, &glyph, em, &advance);
    // The IDWriteFactory2 form takes an explicit antialias mode, so ClearType is requested rather
    // than inferred from the rendering params.
    let analysis = unsafe {
        fonts.factory.CreateGlyphRunAnalysis(
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
    let bounds = unsafe { analysis.GetAlphaTextureBounds(DWRITE_TEXTURE_CLEARTYPE_3x1)? };
    let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    if w <= 0 || h <= 0 {
        return Ok(None);
    }
    let mut buf = vec![0u8; (w * h * 3) as usize];
    unsafe {
        analysis.CreateAlphaTexture(DWRITE_TEXTURE_CLEARTYPE_3x1, &bounds, &mut buf)?;
    }
    Ok(Some((bounds, buf)))
}

/// Monochrome glyph → RGBA8 where RGB are subpixel coverages and A is the greyscale average.
pub fn raster_mono(fonts: &Fonts, face_idx: u16, glyph: u16, em: f32) -> Result<Option<Raster>> {
    let face = fonts.face(face_idx).clone();
    let Some((bounds, cov)) = raster_layer(fonts, &face, glyph, em)? else {
        return Ok(None);
    };
    let (w, h) = (
        (bounds.right - bounds.left) as usize,
        (bounds.bottom - bounds.top) as usize,
    );
    let mut pixels = vec![0u8; w * h * 4];
    for i in 0..w * h {
        let (r, g, b) = (cov[i * 3], cov[i * 3 + 1], cov[i * 3 + 2]);
        let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
        pixels[i * 4] = r;
        pixels[i * 4 + 1] = g;
        pixels[i * 4 + 2] = b;
        pixels[i * 4 + 3] = avg;
    }
    Ok(Some(Raster {
        bounds,
        pixels,
        colour: false,
    }))
}

/// Colour glyph → premultiplied BGRA8, composited from the COLR layers.
///
/// Returns `Ok(None)` if the glyph has no colour layers, in which case the caller should fall back
/// to `raster_mono`.
pub fn raster_colour(
    fonts: &Fonts,
    face_idx: u16,
    glyph: u16,
    em: f32,
    text_colour: [f32; 4],
) -> Result<Option<Raster>> {
    let face = fonts.face(face_idx).clone();
    let advance = 0.0f32;
    let run = make_run(&face, &glyph, em, &advance);

    let enumerator = unsafe {
        fonts.factory.TranslateColorGlyphRun(
            0.0,
            0.0,
            &run,
            None,
            DWRITE_MEASURING_MODE_NATURAL,
            None,
            0,
        )
    };
    let enumerator = match enumerator {
        Ok(e) => e,
        Err(e) if e.code() == DWRITE_E_NOCOLOR => return Ok(None),
        Err(e) => return Err(e),
    };

    // Pass 1: collect every layer's coverage and bounds, and union the bounds.
    let mut layers: Vec<(RECT, Vec<u8>, [f32; 4])> = Vec::new();
    loop {
        let has_more = unsafe { enumerator.MoveNext()? };
        if !has_more.as_bool() {
            break;
        }
        let run_ptr = unsafe { enumerator.GetCurrentRun()? };
        let colour_run = unsafe { &*run_ptr };

        // paletteIndex 0xFFFF means "paint with the text foreground colour".
        let rgba = if colour_run.paletteIndex == 0xFFFF {
            text_colour
        } else {
            let c = colour_run.runColor;
            [c.r, c.g, c.b, c.a]
        };

        // `fontFace` is a ManuallyDrop<Option<..>> owned by DirectWrite. Borrow it — copying it
        // into an owned IDWriteFontFace would Release a reference that was never AddRef'd, and
        // the refcount underflow shows up later as a use-after-free.
        let Some(layer_face) = colour_run.glyphRun.fontFace.as_ref() else {
            continue;
        };
        let count = colour_run.glyphRun.glyphCount as usize;
        if count == 0 || colour_run.glyphRun.glyphIndices.is_null() {
            continue;
        }
        let indices =
            unsafe { std::slice::from_raw_parts(colour_run.glyphRun.glyphIndices, count) };
        for &g in indices {
            if let Some((b, cov)) =
                raster_layer(fonts, layer_face, g, colour_run.glyphRun.fontEmSize)?
            {
                layers.push((b, cov, rgba));
            }
        }
    }

    if layers.is_empty() {
        return Ok(None);
    }

    let mut union = layers[0].0;
    for (b, _, _) in &layers[1..] {
        union.left = union.left.min(b.left);
        union.top = union.top.min(b.top);
        union.right = union.right.max(b.right);
        union.bottom = union.bottom.max(b.bottom);
    }
    let (uw, uh) = (
        (union.right - union.left) as usize,
        (union.bottom - union.top) as usize,
    );

    // Pass 2: composite in premultiplied space, back to front.
    let mut acc = vec![0.0f32; uw * uh * 4];
    for (b, cov, rgba) in &layers {
        let (lw, lh) = (
            (b.right - b.left) as usize,
            (b.bottom - b.top) as usize,
        );
        let (ox, oy) = ((b.left - union.left) as usize, (b.top - union.top) as usize);
        for y in 0..lh {
            for x in 0..lw {
                let i = y * lw + x;
                // A coloured layer cannot carry subpixel coverage — the three channels would have
                // to survive an alpha composite against a different colour. Average to greyscale.
                let c = (cov[i * 3] as f32 + cov[i * 3 + 1] as f32 + cov[i * 3 + 2] as f32)
                    / (3.0 * 255.0);
                if c <= 0.0 {
                    continue;
                }
                let a = c * rgba[3];
                let d = ((oy + y) * uw + ox + x) * 4;
                for ch in 0..3 {
                    acc[d + ch] = rgba[ch] * a + acc[d + ch] * (1.0 - a);
                }
                acc[d + 3] = a + acc[d + 3] * (1.0 - a);
            }
        }
    }

    // BGRA8, premultiplied.
    let mut pixels = vec![0u8; uw * uh * 4];
    for i in 0..uw * uh {
        let to8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        pixels[i * 4] = to8(acc[i * 4 + 2]);
        pixels[i * 4 + 1] = to8(acc[i * 4 + 1]);
        pixels[i * 4 + 2] = to8(acc[i * 4]);
        pixels[i * 4 + 3] = to8(acc[i * 4 + 3]);
    }

    Ok(Some(Raster {
        bounds: union,
        pixels,
        colour: true,
    }))
}
