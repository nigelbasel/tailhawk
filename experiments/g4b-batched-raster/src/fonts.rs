//! Font resolution. Deliberately duplicated from `g4-glyph-atlas` rather than shared — the
//! experiments are independent binaries and a shared crate would couple their build graphs.

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory2, IDWriteFontFace, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
};

pub struct Fonts {
    pub factory: IDWriteFactory2,
    pub face: IDWriteFontFace,
    pub name: String,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Fonts {
    /// Resolves the first available candidate. CJK first, because G4 measured its rasterisation
    /// cost against a cold viewport of CJK glyphs and this experiment must be comparable to it.
    pub fn new(candidates: &[&str]) -> Result<Self> {
        let factory: IDWriteFactory2 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let mut collection = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false)? };
        let collection = collection.expect("system font collection");

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
            return Ok(Self {
                factory,
                face,
                name: (*name).to_string(),
            });
        }
        panic!("none of {candidates:?} is installed");
    }

    /// Bulk codepoint → glyph index. One call for the whole set; this is not the cost under test.
    pub fn glyph_indices(&self, codepoints: &[u32]) -> Vec<u16> {
        let mut out = vec![0u16; codepoints.len()];
        unsafe {
            let _ = self.face.GetGlyphIndices(
                codepoints.as_ptr(),
                codepoints.len() as u32,
                out.as_mut_ptr(),
            );
        }
        out
    }
}
