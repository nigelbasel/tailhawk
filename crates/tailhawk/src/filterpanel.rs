//! §2.1's docked filter panel — the owner's refinement of the classic-dialogs decision, pointing
//! at Visual Studio's tool windows: filters are toggled constantly while reading, so their surface
//! is non-modal, fixed above the status bar in the band the record-detail pane already proved.
//!
//! The shape is TextAnalysisTool.NET's — the tool `SPEC.md` §7.3 names as the model: one row per
//! filter with its enabled mark, polarity and text; the add field beneath them. **No floating and
//! no drag-docking in v1**, per §2.1 as resettled.
//!
//! This module is the pure half: chips in, one frame's rows out, and the band's height as one
//! arithmetic answer asked by both the reserver and the drawer — reserving from one number and
//! drawing from another is how the status bar once ended up a different size from its hole.

use tailhawk_core::filter::{Chip, Polarity};

/// One filter as a frame draws it: the enabled mark, the polarity sign, the text.
///
/// The mark is ASCII deliberately — `☑` is .notdef in the chrome face on some machines, the same
/// trap the header markers and the sheet's `▸` hit; `[x]` cannot fail to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PanelRow {
    pub mark: &'static str,
    pub sign: char,
    pub text: String,
}

/// The chips as the panel's rows, in chip order — order is display-only, per §7.2.
pub fn rows_of(chips: &[Chip]) -> Vec<PanelRow> {
    chips
        .iter()
        .map(|chip| PanelRow {
            mark: if chip.enabled { "[x]" } else { "[ ]" },
            sign: match chip.polarity {
                Polarity::Include => '+',
                Polarity::Exclude => '−',
            },
            text: chip.source.clone(),
        })
        .collect()
}

/// The rule drawn along the panel's top edge, and the air under it.
pub const RULE_PX: f32 = 1.0;
const PAD_PX: f32 = 4.0;

/// The band's height: a row per chip, the add row, the top rule — and nothing when hidden,
/// which is what lets the reserver ask unconditionally.
pub fn height(chips: usize, visible: bool, row_h: f32) -> f32 {
    if !visible {
        return 0.0;
    }
    (chips as f32 + 1.0) * row_h + RULE_PX + PAD_PX
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chip(source: &str, include: bool, enabled: bool) -> Chip {
        let polarity = if include {
            Polarity::Include
        } else {
            Polarity::Exclude
        };
        let mut chip = Chip::parse(source, polarity).expect("a plain word parses");
        chip.enabled = enabled;
        chip
    }

    /// The three facts a row carries, each from the chip it draws.
    #[test]
    fn a_rows_mark_sign_and_text_come_from_its_chip() {
        let rows = rows_of(&[chip("error", true, true), chip("retry", false, false)]);
        assert_eq!(rows[0].mark, "[x]");
        assert_eq!(rows[0].sign, '+');
        assert_eq!(rows[0].text, "error");
        assert_eq!(rows[1].mark, "[ ]", "a disabled chip shows an empty box");
        assert_eq!(rows[1].sign, '−');
    }

    /// The reserver and the drawer ask the same function, so the one thing worth pinning is the
    /// shape: hidden is exactly zero, and every chip adds exactly one row to the add row.
    #[test]
    fn the_band_is_zero_hidden_and_one_row_per_chip_shown() {
        assert_eq!(height(5, false, 20.0), 0.0);
        let empty = height(0, true, 20.0);
        assert_eq!(empty, 20.0 + RULE_PX + 4.0, "the add row alone");
        assert_eq!(height(3, true, 20.0) - empty, 60.0);
    }
}
