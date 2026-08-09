//! Making a colour more vivid without touching the ones the terminal owns.
//!
//! Split from [`super`] for the file-length limit: the colour *model* is one subject, and adjusting
//! a colour is another. The rule that matters lives here — see [`Color::intensified`].

use super::Color;

/// How much a vivid colour gains on each HSV axis.
///
/// One place to tune the palette, rather than twenty hex literals to re-pick by eye.
const SATURATION_GAIN: f32 = 0.18;

/// The saturation below which a colour is treated as having no hue to intensify.
///
/// This is what protects the parts of the palette that are *structure* rather than colour: the
/// dropdown's grey chrome, the black on `sudo`'s red, and white text. Brightening a grey does not
/// make it a better grey, it makes it a worse grey.
const GREY: f32 = 0.15;

impl Color {
    /// The same hue, more vivid — higher saturation, and brighter or deeper.
    ///
    /// **An ANSI slot is returned unchanged, and that is the point.** `Basic` means "colour 2,
    /// whatever the terminal thinks that is", so the user's own scheme — and any tool that remaps
    /// it, pywal and friends — decides what it looks like. Rewriting it to an absolute RGB value
    /// would be oslo overruling a choice the user made somewhere else entirely. Only `Rgb`, which
    /// oslo picked itself, is oslo's to adjust.
    ///
    /// # HSV, not HSL
    ///
    /// Because "brighter" means *more colour*, and in HSL it does not. Raising HSL lightness past
    /// the midpoint bleeds chroma toward white, so `#ff5555` becomes `#ff8888` — a paler red, which
    /// is the opposite of what was asked for. HSV separates the two: value lifts a dark colour, and
    /// saturation sharpens one that is already light. A colour at full value simply gets purer.
    ///
    /// `value` is signed: positive brightens against a dark background, negative deepens against a
    /// light one. Zero still sharpens the hue, which is the useful move when the brightness is
    /// already right.
    pub fn intensified(self, value: f32) -> Color {
        let Color::Rgb(r, g, b) = self else {
            return self;
        };
        let (h, s, v) = to_hsv(r, g, b);
        // No hue means nothing to intensify — see `GREY`.
        if s < GREY {
            return self;
        }
        let s = (s + SATURATION_GAIN).clamp(0.0, 1.0);
        let v = (v + value).clamp(0.05, 1.0);
        let (r, g, b) = from_hsv(h, s, v);
        Color::Rgb(r, g, b)
    }
}

/// RGB to hue (0..360), saturation and value (each 0..1).
fn to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
    );
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let span = max - min;
    let s = if max <= f32::EPSILON { 0.0 } else { span / max };
    if span <= f32::EPSILON {
        return (0.0, s, max);
    }
    let h = if max == r {
        ((g - b) / span) % 6.0
    } else if max == g {
        (b - r) / span + 2.0
    } else {
        (r - g) / span + 4.0
    };
    ((h * 60.0 + 360.0) % 360.0, s, max)
}

/// The inverse.
fn from_hsv(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - (((h / 60.0) % 2.0) - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h {
        h if h < 60.0 => (c, x, 0.0),
        h if h < 120.0 => (x, c, 0.0),
        h if h < 180.0 => (0.0, c, x),
        h if h < 240.0 => (0.0, x, c),
        h if h < 300.0 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |q: f32| ((q + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color) -> String {
        match c {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            other => format!("{other:?}"),
        }
    }

    /// **The rule the whole change rests on: an ANSI slot is never rewritten.**
    ///
    /// `Basic` means "colour 2, whatever this terminal thinks that is". A user's scheme, or a tool
    /// like pywal that remaps the slots, owns that answer; turning it into an absolute RGB value
    /// would be oslo overruling a choice made somewhere else.
    #[test]
    fn ansi_and_indexed_colours_are_left_alone() {
        for colour in [
            Color::Default,
            Color::Basic {
                index: 2,
                bright: false,
            },
            Color::Basic {
                index: 7,
                bright: true,
            },
            Color::Indexed(240),
            Color::Indexed(236),
        ] {
            assert_eq!(colour.intensified(0.5), colour, "{colour:?} was rewritten");
            assert_eq!(colour.intensified(-0.5), colour, "{colour:?} was rewritten");
        }
    }

    /// A colour with no hue has nothing to intensify, so structure survives: the black on `sudo`'s
    /// red, white text, and the dropdown's greys.
    #[test]
    fn greys_black_and_white_are_left_alone() {
        for colour in [
            Color::Rgb(0, 0, 0),
            Color::Rgb(0xff, 0xff, 0xff),
            Color::Rgb(0x62, 0x62, 0x62),
        ] {
            assert_eq!(colour.intensified(0.2), colour, "{} moved", hex(colour));
        }
    }

    /// A lift makes a colour brighter and keeps its hue — a brighter green is still green.
    #[test]
    fn a_lift_brightens_without_moving_the_hue() {
        let green = Color::Rgb(0x50, 0xfa, 0x7b);
        let Color::Rgb(r, g, b) = green.intensified(0.12) else {
            panic!("still rgb");
        };
        let (was_h, was_s, was_v) = to_hsv(0x50, 0xfa, 0x7b);
        let (now_h, now_s, now_v) = to_hsv(r, g, b);
        assert!((was_h - now_h).abs() < 2.0, "hue moved: {was_h} -> {now_h}");
        assert!(now_v >= was_v, "not brighter: {was_v} -> {now_v}");
        assert!(now_s > was_s, "not more saturated: {was_s} -> {now_s}");
    }

    /// Zero on the value axis still sharpens the hue, which is what the light palette wants:
    /// more colour, not more light.
    #[test]
    fn a_zero_lift_still_saturates() {
        let plum = Color::Rgb(0x69, 0x39, 0xb8);
        let (_, was_s, was_v) = to_hsv(0x69, 0x39, 0xb8);
        let Color::Rgb(r, g, b) = plum.intensified(0.0) else {
            panic!("still rgb");
        };
        let (_, now_s, now_v) = to_hsv(r, g, b);
        assert!(now_s > was_s, "not sharper: {was_s} -> {now_s}");
        assert!(
            (now_v - was_v).abs() < 0.02,
            "value moved: {was_v} -> {now_v}"
        );
    }

    /// **The failure that made HSV the model.** In HSL, lifting a colour that is already light
    /// bleeds it toward white: `#ff5555` became `#ff8888`, a paler red. "Brighter" has to mean
    /// more colour, so a lift must never reduce saturation.
    #[test]
    fn brightening_never_washes_a_colour_out() {
        for rgb in [
            (0xffu8, 0x55u8, 0x55u8),
            (0x50, 0xfa, 0x7b),
            (0xf1, 0xfa, 0x8c),
            (0xff, 0x79, 0xc6),
        ] {
            let (_, was_s, _) = to_hsv(rgb.0, rgb.1, rgb.2);
            let Color::Rgb(r, g, b) = Color::Rgb(rgb.0, rgb.1, rgb.2).intensified(0.12) else {
                panic!("still rgb");
            };
            let (_, now_s, _) = to_hsv(r, g, b);
            assert!(
                now_s >= was_s,
                "#{:02x}{:02x}{:02x} lost saturation: {was_s} -> {now_s}",
                rgb.0,
                rgb.1,
                rgb.2
            );
        }
    }

    /// Nothing may run away to white, or every bright colour becomes the same colour.
    #[test]
    fn a_large_lift_stops_short_of_white() {
        let Color::Rgb(r, g, b) = Color::Rgb(0xf1, 0xfa, 0x8c).intensified(5.0) else {
            panic!("still rgb");
        };
        assert_ne!((r, g, b), (255, 255, 255), "washed out to white");
        let (_, s, _) = to_hsv(r, g, b);
        assert!(s > 0.5, "hue lost: {s}");
    }

    /// The round trip has to be faithful, or every colour drifts a little each time.
    #[test]
    fn hsv_round_trips() {
        for colour in [
            (0x50u8, 0xfau8, 0x7bu8),
            (0xff, 0x79, 0xc6),
            (0x8b, 0xe9, 0xfd),
            (0x62, 0x72, 0xa4),
            (0x00, 0x00, 0x00),
            (0xff, 0xff, 0xff),
        ] {
            let (h, s, v) = to_hsv(colour.0, colour.1, colour.2);
            let back = from_hsv(h, s, v);
            assert_eq!(back, colour, "round trip lost {colour:?}");
        }
    }
}
