//! Colours, and what to do when the terminal cannot show them.
//!
//! A theme value may be written three ways, because all three are things people already have in
//! their fingers: a name (`"green"`, `"brightblack"`), a hex triplet (`"#61ffca"`), or a 256-colour
//! index (`"240"`). They are all [`Color`], and they all degrade.
//!
//! **Degradation is the part that matters.** A 24-bit colour sent to a terminal that does not
//! understand `\x1b[38;2;…` is not ignored — it prints as literal digits across the user's prompt.
//! So the depth is decided once from the environment and every colour is emitted at that depth or
//! below: truecolor stays as written, 256 quantises the hex to the xterm cube, and 16 folds it
//! down to the nearest basic colour. A theme written for a modern terminal therefore still works
//! over a serial console, which for a distro's `/bin/sh` is not a hypothetical.

use std::fmt::Write as _;

/// How much colour this terminal can be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Depth {
    /// No colour at all: `$TERM` is `dumb`, or `$NO_COLOR` is set.
    None,
    /// The eight basic colours and their bright forms.
    Ansi16,
    /// The xterm 256-colour palette.
    Ansi256,
    /// 24-bit `\x1b[38;2;r;g;b`.
    True,
}

impl Depth {
    /// A depth by the name a config writes, or `None` for anything else.
    ///
    /// The aliases are the ones people already type: `truecolor` and `24bit` are the same thing,
    /// and `off` reads better than `none` next to a `= ` sign.
    pub fn named(name: &str) -> Option<Depth> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "truecolor" | "24bit" | "24-bit" | "true" => Depth::True,
            "256" | "ansi256" | "8bit" => Depth::Ansi256,
            "16" | "ansi16" | "8" | "basic" => Depth::Ansi16,
            "none" | "off" | "no" | "mono" => Depth::None,
            _ => return None,
        })
    }

    /// What the environment says this terminal can do.
    ///
    /// `$NO_COLOR` wins over everything — it is the one convention whose whole point is that a
    /// program does not get to argue. After that `$COLORTERM` is the only reliable signal for
    /// 24-bit; `$TERM` is consulted for the rest, because a `TERM` naming 256 colours is the
    /// closest thing to a promise a terminal makes.
    pub fn detect() -> Depth {
        if std::env::var_os("NO_COLOR").is_some() {
            return Depth::None;
        }
        let term = std::env::var("TERM").unwrap_or_default();
        if term.is_empty() || term == "dumb" {
            return Depth::None;
        }
        match std::env::var("COLORTERM").unwrap_or_default().as_str() {
            "truecolor" | "24bit" => return Depth::True,
            _ => {}
        }
        if term.contains("256") || term.contains("direct") {
            return Depth::Ansi256;
        }
        Depth::Ansi16
    }
}

/// One colour, in whichever form it was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// Whatever the terminal's default foreground or background is.
    Default,
    /// A basic colour, 0..=7, plus a bright flag.
    Basic {
        index: u8,
        bright: bool,
    },
    /// An xterm palette index.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    /// Read a theme value: `"green"`, `"brightblack"`, `"#61ffca"`, `"240"`.
    ///
    /// `None` for anything unrecognised rather than a silent black: a typo in a theme should show
    /// up as "that element is not coloured", which is visible, rather than as a colour the user
    /// did not choose.
    pub fn parse(text: &str) -> Option<Color> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        if let Some(hex) = text.strip_prefix('#') {
            return parse_hex(hex);
        }
        if let Ok(index) = text.parse::<u8>() {
            return Some(Color::Indexed(index));
        }

        let lower = text.to_ascii_lowercase();
        let (name, bright) = match lower.strip_prefix("bright") {
            // Both spellings, because fish writes `brblack` and everyone else writes
            // `brightblack`.
            Some(rest) => (rest.trim_start_matches('-').to_string(), true),
            None => match lower.strip_prefix("br") {
                Some(rest) if BASIC.contains(&rest) => (rest.to_string(), true),
                _ => (lower.clone(), false),
            },
        };
        if name == "normal" || name == "default" {
            return Some(Color::Default);
        }
        BASIC
            .iter()
            .position(|b| *b == name)
            .map(|index| Color::Basic {
                index: index as u8,
                bright,
            })
    }

    /// The SGR parameters for this colour, at `depth`, as a foreground or a background.
    fn sgr(self, depth: Depth, background: bool) -> Option<String> {
        if depth == Depth::None {
            return None;
        }
        let base = if background { 40 } else { 30 };
        Some(match self.at(depth) {
            Color::Default => format!("{}", base + 9),
            Color::Basic { index, bright } => {
                // The bright range is 90/100, which every terminal that has colour understands —
                // there is no need to fall back to the bold-as-bright trick.
                let start = if bright { base + 60 } else { base };
                format!("{}", start + index as u32)
            }
            Color::Indexed(i) => format!("{};5;{i}", base + 8),
            Color::Rgb(r, g, b) => format!("{};2;{r};{g};{b}", base + 8),
        })
    }

    /// This colour expressed within what `depth` can show.
    fn at(self, depth: Depth) -> Color {
        match (self, depth) {
            (Color::Rgb(r, g, b), Depth::Ansi256) => Color::Indexed(quantise_256(r, g, b)),
            (Color::Rgb(r, g, b), Depth::Ansi16) => nearest_basic(r, g, b),
            (Color::Indexed(i), Depth::Ansi16) => {
                let (r, g, b) = from_256(i);
                nearest_basic(r, g, b)
            }
            (colour, _) => colour,
        }
    }
}

const BASIC: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

fn parse_hex(hex: &str) -> Option<Color> {
    // `#abc` is the CSS short form, and people write it.
    let full = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => hex.to_string(),
        _ => return None,
    };
    let value = u32::from_str_radix(&full, 16).ok()?;
    Some(Color::Rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
}

/// An RGB triplet as an xterm-256 index.
///
/// The palette is 16 system colours, a 6×6×6 cube, then 24 greys. Greys are checked first because
/// the cube's grey diagonal is coarse — quantising `#808080` into the cube gives a visible tint,
/// where the grey ramp has it almost exactly.
fn quantise_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return 232 + ((r as u16 - 8) * 24 / 247) as u8;
    }
    let step = |v: u8| -> u16 {
        // The cube's levels are 0, 95, 135, 175, 215, 255 — not evenly spaced, which is why this
        // is a table rather than a division.
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, level)| (**level as i16 - v as i16).abs())
            .map(|(i, _)| i as u16)
            .unwrap_or(0)
    };
    (16 + 36 * step(r) + 6 * step(g) + step(b)) as u8
}

/// The centre of an xterm-256 index, for folding further down to 16.
fn from_256(index: u8) -> (u8, u8, u8) {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match index {
        0..=15 => {
            let bright = index >= 8;
            let i = index % 8;
            let on = if bright { 255 } else { 128 };
            (
                if i & 1 != 0 { on } else { 0 },
                if i & 2 != 0 { on } else { 0 },
                if i & 4 != 0 { on } else { 0 },
            )
        }
        16..=231 => {
            let i = index - 16;
            (
                LEVELS[(i / 36) as usize],
                LEVELS[((i % 36) / 6) as usize],
                LEVELS[(i % 6) as usize],
            )
        }
        _ => {
            let grey = 8 + (index as u16 - 232) * 10;
            (grey as u8, grey as u8, grey as u8)
        }
    }
}

/// The nearest of the sixteen basic colours.
fn nearest_basic(r: u8, g: u8, b: u8) -> Color {
    let mut best = (u32::MAX, 0u8, false);
    for bright in [false, true] {
        for index in 0..8u8 {
            let on = if bright { 255i32 } else { 128i32 };
            let (cr, cg, cb) = (
                if index & 1 != 0 { on } else { 0 },
                if index & 2 != 0 { on } else { 0 },
                if index & 4 != 0 { on } else { 0 },
            );
            let d = (cr - r as i32).pow(2) + (cg - g as i32).pow(2) + (cb - b as i32).pow(2);
            if (d as u32) < best.0 {
                best = (d as u32, index, bright);
            }
        }
    }
    Color::Basic {
        index: best.1,
        bright: best.2,
    }
}

/// A colour plus the attributes that go with it.
///
/// Every theme entry is one of these, whether it was written as `"green"` or as
/// `{fg = "green", bold = true}` — the short form is just a `Style` with only `fg` set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub reverse: bool,
}

impl Style {
    pub fn fg(colour: Color) -> Style {
        Style {
            fg: Some(colour),
            ..Style::default()
        }
    }

    /// Whether this style would emit nothing at all.
    pub fn is_plain(&self) -> bool {
        *self == Style::default()
    }

    /// The escape sequence that turns this style on, at `depth`.
    ///
    /// Empty when the style is plain or the terminal has no colour, so a caller can concatenate
    /// unconditionally without checking — and a `NO_COLOR` run emits a line with no escapes in it
    /// at all rather than a stream of empty `\x1b[m`.
    pub fn open(&self, depth: Depth) -> String {
        if depth == Depth::None || self.is_plain() {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.bold {
            parts.push("1".into());
        }
        if self.dim {
            parts.push("2".into());
        }
        if self.italic {
            parts.push("3".into());
        }
        if self.underline {
            parts.push("4".into());
        }
        if self.reverse {
            parts.push("7".into());
        }
        if let Some(fg) = self.fg.and_then(|c| c.sgr(depth, false)) {
            parts.push(fg);
        }
        if let Some(bg) = self.bg.and_then(|c| c.sgr(depth, true)) {
            parts.push(bg);
        }
        if parts.is_empty() {
            return String::new();
        }
        let mut out = String::from("\x1b[");
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                out.push(';');
            }
            let _ = write!(out, "{part}");
        }
        out.push('m');
        out
    }

    /// `text` wrapped in this style, reset afterwards.
    pub fn paint(&self, text: &str, depth: Depth) -> String {
        let open = self.open(depth);
        if open.is_empty() {
            return text.to_string();
        }
        format!("{open}{text}\x1b[0m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_can_be_written_three_ways() {
        assert_eq!(Color::parse("#61ffca"), Some(Color::Rgb(0x61, 0xff, 0xca)));
        // The CSS short form expands by doubling each digit.
        assert_eq!(Color::parse("#0f8"), Some(Color::Rgb(0x00, 0xff, 0x88)));
        assert_eq!(Color::parse("240"), Some(Color::Indexed(240)));
        assert_eq!(
            Color::parse("green"),
            Some(Color::Basic {
                index: 2,
                bright: false
            })
        );
        assert_eq!(
            Color::parse("brightblack"),
            Some(Color::Basic {
                index: 0,
                bright: true
            })
        );
        // fish's spelling of the same thing.
        assert_eq!(Color::parse("brblack"), Color::parse("brightblack"));
        assert_eq!(Color::parse("normal"), Some(Color::Default));
    }

    /// A typo is uncoloured, not black. A black-on-black prompt is far harder to diagnose than an
    /// element that simply did not take its colour.
    #[test]
    fn an_unrecognised_value_is_refused_rather_than_guessed() {
        assert_eq!(Color::parse("chartreuse"), None);
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse(""), None);
    }

    /// A 24-bit colour sent to a 256-colour terminal prints as digits across the prompt, so every
    /// colour has to come down to what the terminal admits to.
    #[test]
    fn colours_degrade_to_what_the_terminal_can_show() {
        let teal = Style::fg(Color::Rgb(0x61, 0xff, 0xca));
        assert!(teal.open(Depth::True).contains("38;2;97;255;202"));
        assert!(teal.open(Depth::Ansi256).contains("38;5;"));
        assert!(!teal.open(Depth::Ansi256).contains("38;2;"));
        // At 16 colours it is one of the eight, which for this teal is bright cyan.
        assert_eq!(teal.open(Depth::Ansi16), "\x1b[96m");
        // And nothing at all when there is no colour to be had.
        assert_eq!(teal.open(Depth::None), "");
        assert_eq!(teal.paint("x", Depth::None), "x");
    }

    /// Grey is the case a naive cube quantiser gets visibly wrong, so it has its own ramp.
    #[test]
    fn greys_use_the_grey_ramp_rather_than_the_colour_cube() {
        let index = quantise_256(0x80, 0x80, 0x80);
        assert!(
            (232..=255).contains(&index),
            "mid grey quantised to {index}, outside the grey ramp"
        );
    }

    #[test]
    fn attributes_come_before_the_colour() {
        let style = Style {
            fg: Some(Color::Basic {
                index: 1,
                bright: false,
            }),
            bold: true,
            underline: true,
            ..Style::default()
        };
        assert_eq!(style.open(Depth::Ansi16), "\x1b[1;4;31m");
        assert_eq!(style.paint("no", Depth::Ansi16), "\x1b[1;4;31mno\x1b[0m");
    }

    /// A plain style emits nothing, so callers can concatenate without testing first.
    #[test]
    fn a_plain_style_is_silent() {
        assert_eq!(Style::default().open(Depth::True), "");
        assert_eq!(Style::default().paint("x", Depth::True), "x");
    }
}

/// Making a colour more vivid without touching the ones the terminal owns.
mod vivid;
