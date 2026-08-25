//! The shared standard-controls module — the owner's directive, 2026-08-25: "implement a
//! standard set of classes that replicate the aspects of the WinUI libraries that you can just
//! use. This redrawing is eating time, and is wrong every time I look at it."
//!
//! Chrome that must live inside the rendered window — the filter panel, the status bar — draws
//! its controls through here and nowhere else, so metrics, colours and behaviour are decided
//! once. Everything native stays native: menus are `HMENU`, dialogs are `DLGTEMPLATE`; this
//! module exists only for the surfaces that cannot be.
//!
//! The pure half is the metrics — paddings and sizes as one set of answers, testable — and the
//! drawing half is thin calls into the painter that return the hit rectangle, because a control
//! that is drawn but cannot be clicked is not a control.

use tailhawk_core::paint::Painter;
use tailhawk_core::theme::theme;

/// The standard paddings, in chrome-face pixels. One place, because every surface that invented
/// its own is a surface the owner caught looking wrong.
pub struct Metrics {
    /// Horizontal padding inside a button, either side of its label.
    pub button_pad_x: f32,
    /// Vertical padding inside a button, above and below its label.
    pub button_pad_y: f32,
    /// The gap between neighbouring controls in a row.
    pub gap: f32,
}

/// The metrics for a chrome line height — proportional, so the controls scale with the face.
pub fn metrics(chrome_h: f32) -> Metrics {
    Metrics {
        button_pad_x: (chrome_h * 0.55).max(6.0),
        button_pad_y: (chrome_h * 0.15).max(2.0),
        gap: (chrome_h * 0.35).max(4.0),
    }
}

/// A button's size for its label, from the same numbers the drawing uses.
pub fn button_size(painter: &mut Painter, chrome_h: f32, label: &str) -> (f32, f32) {
    let m = metrics(chrome_h);
    (
        painter.chrome_measure(label) + m.button_pad_x * 2.0,
        chrome_h + m.button_pad_y * 2.0,
    )
}

/// Draws a standard text button with its top-left at `(x, y)` and returns its rectangle for the
/// hit map. The look is the classic flat-bordered button both themes carry: a field fill, a
/// one-pixel border, the label centred.
pub fn button(
    painter: &mut Painter,
    chrome_h: f32,
    x: f32,
    y: f32,
    label: &str,
    enabled: bool,
) -> (core::ops::Range<f32>, core::ops::Range<f32>) {
    let m = metrics(chrome_h);
    let (w, h) = button_size(painter, chrome_h, label);
    painter.fill(x, y, w, h, theme().field_bg);
    let edge = theme().pane_edge;
    painter.fill(x, y, w, 1.0, edge);
    painter.fill(x, y + h - 1.0, w, 1.0, edge);
    painter.fill(x, y, 1.0, h, edge);
    painter.fill(x + w - 1.0, y, 1.0, h, edge);
    let ink = if enabled {
        theme().ink
    } else {
        theme().field_hint
    };
    painter.chrome_run(label, x + m.button_pad_x, y + m.button_pad_y, ink);
    (x..x + w, y..y + h)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property worth pinning without a device: the metrics scale with the face and
    /// never collapse below the click-target minimums.
    #[test]
    fn the_metrics_scale_and_never_collapse() {
        let small = metrics(4.0);
        assert_eq!(small.button_pad_x, 6.0, "the floor holds at tiny faces");
        assert_eq!(small.gap, 4.0);
        let big = metrics(40.0);
        assert!(big.button_pad_x > small.button_pad_x);
        assert!(big.gap > small.gap);
        assert!(big.button_pad_y > small.button_pad_y);
    }
}
