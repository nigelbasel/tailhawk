//! ANSI escape sanitisation — `SPEC.md` §13.4, E24.
//!
//! §13.4: "CSI sequences are stripped before format matching (MEL's Simple formatter colours by
//! default, so files captured via `tee`, Docker or a PTY carry escapes that break every regex). If
//! ANSI rendering is enabled, **only SGR colour and intensity are honoured** — never OSC 8
//! hyperlinks, never OSC 52 clipboard, never title-setting, never DCS."
//!
//! ## Removed at decode, so every reader of a line agrees
//!
//! [`strip`] is applied by [`LineDecoder`](crate::lines::LineDecoder) to every line it emits, and
//! that decoder is the one path the viewport, the search and the filter all read through. So a
//! match's byte offsets, a chip's verdict and the painted row describe **the same text**. Stripping
//! only for display would put search hits at offsets the painter could not find, and stripping only
//! for search would highlight the wrong characters. §5.6's "copy preserves the original bytes"
//! therefore does not reach escapes: what is copied is what is shown, which is also what a terminal
//! copies.
//!
//! ## What is recognised, from ECMA-48
//!
//! - **CSI** — `ESC [`, parameter bytes `0x30–0x3F`, intermediate bytes `0x20–0x2F`, one final byte
//!   `0x40–0x7E`. Removed whole. A final byte of `m` is **SGR**, and its colour and intensity
//!   parameters are read into [`Span`]s over the text that follows, in the offsets of the *stripped*
//!   line — the only offsets anything downstream has.
//! - **OSC** `ESC ]`, **DCS** `ESC P`, **SOS** `ESC X`, **PM** `ESC ^`, **APC** `ESC _` — removed
//!   through their **ST** (`ESC \`), or BEL for OSC as every terminal accepts. Never interpreted:
//!   this is where hyperlinks, clipboard writes and title changes live, and §13.4 names each.
//! - **Two-byte `ESC x`** for `x` in `0x30–0x7E` (except the introducers above; ECMA-48 §5.4 puts the private and standard escapes together in that range), and the
//!   three-byte charset designations `ESC ( x` and their siblings — removed.
//! - A **lone `ESC`** with nothing after it, or followed by a byte outside those ranges, is
//!   removed on its own and the byte after it kept. It is a control character §5.6 says must not
//!   silently vanish; here it is not silent, because a revealed line (`CellModel::reveal_invisibles`)
//!   would show what remained. Recorded as the one place §5.6 and §13.4 pull in different directions
//!   and §13.4 wins: an `ESC` that reaches a regex is what breaks it.
//!
//! An unterminated sequence at the end of a line — a writer cut off mid-escape — is removed to the
//! end, which is the same thing a terminal would show for it.
//!
//! ## SGR is parsed but not yet drawn
//!
//! [`Stripped::spans`] carries the colours, and nothing paints them yet: §13.4 makes ANSI rendering
//! a toggle, there is no settings surface to hold one (§12 is M8), and the default that costs
//! nothing when it is wrong is *off*. The palette below is ours and provisional with the rest.

use crate::highlight::{Colour, Span};

/// The 16 SGR colours, normal then bright, on the dark ground. Provisional; §11.2 pins no hex.
pub const PALETTE: [Colour; 16] = [
    [0.35, 0.37, 0.40, 1.0],
    [0.90, 0.40, 0.36, 1.0],
    [0.55, 0.80, 0.50, 1.0],
    [0.88, 0.75, 0.40, 1.0],
    [0.45, 0.65, 0.92, 1.0],
    [0.80, 0.55, 0.85, 1.0],
    [0.45, 0.80, 0.80, 1.0],
    [0.80, 0.82, 0.85, 1.0],
    [0.50, 0.53, 0.58, 1.0],
    [0.98, 0.52, 0.48, 1.0],
    [0.65, 0.90, 0.60, 1.0],
    [0.98, 0.86, 0.50, 1.0],
    [0.58, 0.76, 1.00, 1.0],
    [0.92, 0.66, 0.96, 1.0],
    [0.55, 0.92, 0.92, 1.0],
    [0.95, 0.96, 0.98, 1.0],
];

/// Whether `text` holds anything [`strip`] would change — a byte scan, so the common line costs
/// exactly that and no copy.
pub fn has_escapes(text: &str) -> bool {
    text.as_bytes().contains(&0x1B)
}

/// A line with its escapes removed, and the colours the SGR sequences among them asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stripped {
    pub text: String,
    /// In byte offsets of [`text`](Self::text), sorted and non-overlapping.
    pub spans: Vec<Span>,
}

/// Removes every escape sequence from `text` into `out`, reading SGR colour along the way.
///
/// `out` is cleared and reused. The escapes are removed whether or not anyone wants the spans; the
/// spans are what §13.4 permits a renderer to honour, and nothing else about a sequence survives.
pub fn strip(text: &str, out: &mut Stripped) {
    out.text.clear();
    out.spans.clear();
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut state = Sgr::default();
    let mut open_at = 0usize;
    let mut plain_from = 0usize;

    while i < bytes.len() {
        if bytes[i] != 0x1B {
            i += 1;
            continue;
        }
        // Everything before the escape is text.
        out.text.push_str(&text[plain_from..i]);
        let (next, sgr) = sequence(bytes, i);
        if let Some(params) = sgr {
            let before = state;
            state.apply(&text[params]);
            if state != before {
                state_span(&mut out.spans, before, open_at, out.text.len());
                open_at = out.text.len();
            }
        }
        i = next;
        plain_from = i.min(bytes.len());
    }
    out.text.push_str(&text[plain_from.min(text.len())..]);
    state_span(&mut out.spans, state, open_at, out.text.len());
}

/// Consumes one escape sequence starting at `at` (an `ESC`). Returns the index just past it, and
/// the byte range of the parameters if it was SGR.
fn sequence(bytes: &[u8], at: usize) -> (usize, Option<core::ops::Range<usize>>) {
    let Some(&intro) = bytes.get(at + 1) else {
        return (at + 1, None);
    };
    match intro {
        b'[' => {
            let mut i = at + 2;
            let params_from = i;
            while i < bytes.len() && (0x30..=0x3F).contains(&bytes[i]) {
                i += 1;
            }
            let params_to = i;
            while i < bytes.len() && (0x20..=0x2F).contains(&bytes[i]) {
                i += 1;
            }
            match bytes.get(i) {
                Some(&fin) if (0x40..=0x7E).contains(&fin) => {
                    let sgr = (fin == b'm').then_some(params_from..params_to);
                    (i + 1, sgr)
                }
                // Unterminated, or broken by a byte outside the grammar: drop what we have.
                _ => (i, None),
            }
        }
        b']' | b'P' | b'X' | b'^' | b'_' => {
            let mut i = at + 2;
            while i < bytes.len() {
                if bytes[i] == 0x07 && intro == b']' {
                    return (i + 1, None);
                }
                if bytes[i] == 0x1B && bytes.get(i + 1) == Some(&b'\\') {
                    return (i + 2, None);
                }
                i += 1;
            }
            (bytes.len(), None)
        }
        // A charset designation takes one final byte, `0x30–0x7E`; anything else — a multi-byte
        // character in particular — is not part of the sequence and must not be split.
        b'(' | b')' | b'*' | b'+' | b'-' | b'.' | b'/' => match bytes.get(at + 2) {
            Some(fin) if (0x30..=0x7E).contains(fin) => (at + 3, None),
            _ => (at + 2, None),
        },
        0x30..=0x7E => (at + 2, None),
        _ => (at + 1, None),
    }
}

/// The colour state SGR parameters build up. `Default` is "no colour asked for".
#[derive(Copy, Clone, Debug, Default, PartialEq)]
struct Sgr {
    fg: Option<Colour>,
    bg: Option<Colour>,
    bold: bool,
}

impl Sgr {
    /// Applies one SGR parameter string — `1;31`, `38;5;208`, `` (which is `0`).
    fn apply(&mut self, params: &str) {
        let mut ps = params
            .split(';')
            .map(|p| {
                p.parse::<u16>()
                    .unwrap_or(if p.is_empty() { 0 } else { u16::MAX })
            })
            .peekable();
        if params.is_empty() {
            *self = Sgr::default();
            return;
        }
        while let Some(p) = ps.next() {
            match p {
                0 => *self = Sgr::default(),
                1 => self.bold = true,
                2 | 22 => self.bold = false,
                30..=37 => self.fg = Some(PALETTE[(p - 30) as usize]),
                90..=97 => self.fg = Some(PALETTE[(p - 90 + 8) as usize]),
                39 => self.fg = None,
                40..=47 => self.bg = Some(PALETTE[(p - 40) as usize]),
                100..=107 => self.bg = Some(PALETTE[(p - 100 + 8) as usize]),
                49 => self.bg = None,
                38 | 48 => {
                    let colour = match ps.next() {
                        Some(5) => ps.next().map(indexed),
                        Some(2) => match (ps.next(), ps.next(), ps.next()) {
                            (Some(r), Some(g), Some(b)) => Some(rgb(r, g, b)),
                            _ => None,
                        },
                        _ => None,
                    };
                    if p == 38 {
                        self.fg = colour;
                    } else {
                        self.bg = colour;
                    }
                }
                _ => {}
            }
        }
    }

    /// The span colours for text under this state. Bold brightens a normal-intensity palette
    /// colour, which is the one thing "intensity" means to a colour renderer.
    fn colours(self) -> (Option<Colour>, Option<Colour>) {
        let fg = match self.fg {
            Some(c) if self.bold => Some(
                PALETTE
                    .iter()
                    .position(|p| *p == c)
                    .filter(|i| *i < 8)
                    .map_or(c, |i| PALETTE[i + 8]),
            ),
            other => other,
        };
        (fg, self.bg)
    }
}

/// The 256-colour cube and greyscale ramp, as xterm lays them out.
fn indexed(n: u16) -> Colour {
    match n {
        0..=15 => PALETTE[n as usize],
        16..=231 => {
            let n = n - 16;
            let level = |v: u16| {
                if v == 0 {
                    0.0
                } else {
                    (55.0 + 40.0 * v as f32) / 255.0
                }
            };
            [level(n / 36), level(n / 6 % 6), level(n % 6), 1.0]
        }
        _ => {
            let g = (8 + 10 * (n.min(255) - 232)) as f32 / 255.0;
            [g, g, g, 1.0]
        }
    }
}

fn rgb(r: u16, g: u16, b: u16) -> Colour {
    [
        r.min(255) as f32 / 255.0,
        g.min(255) as f32 / 255.0,
        b.min(255) as f32 / 255.0,
        1.0,
    ]
}

fn state_span(spans: &mut Vec<Span>, state: Sgr, from: usize, to: usize) {
    let (fg, bg) = state.colours();
    if from < to && (fg.is_some() || bg.is_some()) {
        spans.push(Span {
            start: from,
            end: to,
            fg,
            bg,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripped(text: &str) -> Stripped {
        let mut out = Stripped::default();
        strip(text, &mut out);
        out
    }

    #[test]
    fn a_line_without_escapes_is_untouched_and_costs_no_copy() {
        assert!(!has_escapes(
            "2026-08-17 INFO plain [brackets] and ; semicolons"
        ));
        let out = stripped("plain");
        assert_eq!(out.text, "plain");
        assert!(out.spans.is_empty());
    }

    /// The MEL Simple console line, as `docker logs` writes it.
    #[test]
    fn sgr_colour_is_stripped_and_read_into_spans_over_the_stripped_text() {
        let out = stripped("\x1b[41m\x1b[30mfail\x1b[39m\x1b[22m\x1b[49m: Api.Dispatch[0]");
        assert_eq!(out.text, "fail: Api.Dispatch[0]");
        assert_eq!(out.spans.len(), 1);
        assert_eq!(
            (out.spans[0].start, out.spans[0].end),
            (0, 4),
            "offsets are in the stripped text"
        );
        assert_eq!(out.spans[0].bg, Some(PALETTE[1]));
        assert_eq!(out.spans[0].fg, Some(PALETTE[0]));
    }

    #[test]
    fn bold_brightens_and_reset_ends_a_span() {
        let out = stripped("\x1b[1;32mok\x1b[0m done \x1b[38;5;208mx\x1b[m \x1b[38;2;10;20;30my");
        assert_eq!(out.text, "ok done x y");
        assert_eq!(out.spans.len(), 3);
        assert_eq!(
            out.spans[0].fg,
            Some(PALETTE[10]),
            "bold green is bright green"
        );
        assert_eq!((out.spans[1].start, out.spans[1].end), (8, 9));
        assert_eq!(out.spans[1].fg, Some(indexed(208)));
        assert_eq!(out.spans[2].fg, Some(rgb(10, 20, 30)));
    }

    /// §13.4 names these by name: none is interpreted, all are removed, the text between them
    /// stays.
    #[test]
    fn osc_dcs_and_friends_are_removed_whole_and_never_interpreted() {
        assert_eq!(
            stripped("see \x1b]8;;https://evil.example\x1b\\here\x1b]8;;\x1b\\ now").text,
            "see here now"
        );
        assert_eq!(
            stripped("a\x1b]0;new title\x07b").text,
            "ab",
            "OSC ended by BEL"
        );
        assert_eq!(
            stripped("a\x1b]52;c;ZXZpbA==\x1b\\b").text,
            "ab",
            "OSC 52 clipboard"
        );
        assert_eq!(stripped("a\x1bPq#0;2;0;0;0#0~~\x1b\\b").text, "ab", "DCS");
        assert_eq!(stripped("a\x1b_appb\x1b\\c").text, "ac", "APC");
    }

    #[test]
    fn other_csi_two_byte_and_charset_sequences_are_removed() {
        assert_eq!(
            stripped("\x1b[2J\x1b[H\x1b[?25lprompt\x1b[K").text,
            "prompt"
        );
        assert_eq!(
            stripped("\x1b(Bascii \x1bcreset \x1b7save").text,
            "ascii reset save"
        );
        assert_eq!(stripped("\x1b[1;1H\x1b[0;33mwarn").text, "warn");
    }

    #[test]
    fn a_broken_or_unterminated_sequence_is_dropped_to_where_it_breaks() {
        assert_eq!(
            stripped("cut \x1b[31").text,
            "cut ",
            "unterminated CSI at end of line"
        );
        assert_eq!(stripped("cut \x1b]title with no end").text, "cut ");
        assert_eq!(
            stripped("bad \x1b[31\u{e9}m x").text,
            "bad \u{e9}m x",
            "non-grammar byte ends it"
        );
        assert_eq!(stripped("lone \x1b at end \x1b").text, "lone  at end ");
    }

    #[test]
    fn utf8_around_escapes_survives_intact() {
        let out = stripped("héllo \x1b[32m→ wörld\x1b[0m ✓");
        assert_eq!(out.text, "héllo → wörld ✓");
        assert_eq!(&out.text[out.spans[0].start..out.spans[0].end], "→ wörld");
        assert_eq!(
            stripped("(é and [3é").text,
            "é and é",
            "a sequence never swallows part of a character after it"
        );
    }
}
