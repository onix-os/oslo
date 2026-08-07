//! `ui style` — text with a colour, a border and some room around it.
//!
//! The one widget here that asks nothing. It exists because the alternative is scripts carrying
//! their own `\033[1;35m`, and every script that does becomes a place where the shell's theme
//! stops applying.
//!
//! Output goes to **stdout**, unlike every other widget in this module — it is not a prompt, it is
//! the script's own output, and `ui style hello > file` should write the styled text.

use crate::ui::dropdown::width::pad_to_width;
use crate::ui::prompt::printed_width;
use crate::ui::theme::{self, Style};

/// What to draw around the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Border {
    None,
    /// `┌─┐` — the one that reads as a box without shouting.
    Rounded,
    Square,
    Double,
    Thick,
}

impl Border {
    /// `parse` answers `None` for a name nobody meant, so a typo is a diagnostic rather than a
    /// silently unbordered box.
    pub fn parse(name: &str) -> Option<Border> {
        Some(match name {
            "none" => Border::None,
            "rounded" | "round" => Border::Rounded,
            "square" | "normal" => Border::Square,
            "double" => Border::Double,
            "thick" | "heavy" => Border::Thick,
            _ => return None,
        })
    }

    /// Corners and edges: top-left, top-right, bottom-left, bottom-right, horizontal, vertical.
    pub(super) fn glyphs(self) -> Option<[&'static str; 8]> {
        Some(match self {
            Border::None => return None,
            Border::Rounded => ["╭", "╮", "╰", "╯", "─", "│", "├", "┤"],
            Border::Square => ["┌", "┐", "└", "┘", "─", "│", "├", "┤"],
            Border::Double => ["╔", "╗", "╚", "╝", "═", "║", "╠", "╣"],
            Border::Thick => ["┏", "┓", "┗", "┛", "━", "┃", "┣", "┫"],
        })
    }
}

/// How to draw it.
#[derive(Debug, Clone)]
pub struct Styling {
    pub text: String,
    pub style: Style,
    pub border: Border,
    /// Colour of the border itself, which is usually not the colour of the text.
    pub border_style: Style,
    /// Blank columns each side of the text, inside the border.
    pub padding_x: usize,
    /// Blank rows above and below.
    pub padding_y: usize,
    /// Pad every line to this width. `None` uses the widest line.
    pub width: Option<usize>,
}

impl Default for Styling {
    fn default() -> Self {
        Styling {
            text: String::new(),
            style: Style::default(),
            border: Border::None,
            border_style: theme::current().ui.accent,
            padding_x: 0,
            padding_y: 0,
            width: None,
        }
    }
}

/// Render `spec` as the string to print.
///
/// A pure function of its input: no terminal is touched, nothing is asked, and the result can be
/// captured, redirected or compared in a test.
pub fn style(spec: &Styling) -> String {
    let depth = theme::depth();
    let lines: Vec<&str> = spec.text.split('\n').collect();
    let widest = lines.iter().map(|l| printed_width(l)).max().unwrap_or(0);
    let inner = spec.width.unwrap_or(widest).max(widest) + spec.padding_x * 2;

    let mut body: Vec<String> = Vec::new();
    for _ in 0..spec.padding_y {
        body.push(" ".repeat(inner));
    }
    for line in &lines {
        let padded = format!(
            "{}{}{}",
            " ".repeat(spec.padding_x),
            line,
            " ".repeat(spec.padding_x)
        );
        body.push(pad_to_width(&padded, inner));
    }
    for _ in 0..spec.padding_y {
        body.push(" ".repeat(inner));
    }

    let Some([tl, tr, bl, br, h, v, ..]) = spec.border.glyphs() else {
        // No border: the text is styled and padded and nothing else. Each line separately, so a
        // multi-line string does not carry one escape across a newline — which is what makes a
        // styled block survive being piped through `head`.
        return body
            .iter()
            .map(|line| spec.style.paint(line, depth))
            .collect::<Vec<_>>()
            .join("\n");
    };

    let edge = h.repeat(inner);
    let mut out = vec![spec.border_style.paint(&format!("{tl}{edge}{tr}"), depth)];
    for line in &body {
        out.push(format!(
            "{}{}{}",
            spec.border_style.paint(v, depth),
            spec.style.paint(line, depth),
            spec.border_style.paint(v, depth)
        ));
    }
    out.push(spec.border_style.paint(&format!("{bl}{edge}{br}"), depth));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escapes stripped, so a test can assert on shape rather than on the theme.
    fn plain(rendered: &str) -> String {
        let mut out = String::new();
        let mut chars = rendered.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_border_surrounds_the_text() {
        let out = plain(&style(&Styling {
            text: "hi".to_string(),
            border: Border::Rounded,
            ..Styling::default()
        }));
        assert_eq!(out, "╭──╮\n│hi│\n╰──╯", "{out:?}");
    }

    /// Every line is the same width, or the right edge of the box is ragged.
    #[test]
    fn lines_are_padded_to_the_widest() {
        let out = plain(&style(&Styling {
            text: "short\nmuch longer".to_string(),
            border: Border::Square,
            ..Styling::default()
        }));
        let widths: Vec<usize> = out.lines().map(printed_width).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged box: {widths:?} in {out:?}"
        );
    }

    #[test]
    fn padding_adds_room_inside_the_border() {
        let out = plain(&style(&Styling {
            text: "x".to_string(),
            border: Border::Square,
            padding_x: 2,
            padding_y: 1,
            ..Styling::default()
        }));
        assert_eq!(
            out.lines().count(),
            5,
            "two padding rows plus three: {out:?}"
        );
        assert_eq!(printed_width(out.lines().next().unwrap()), 7, "{out:?}");
    }

    /// Without a border there is no box, just the text — and a multi-line string keeps its lines.
    #[test]
    fn no_border_means_no_box() {
        let out = plain(&style(&Styling {
            text: "a\nb".to_string(),
            ..Styling::default()
        }));
        assert_eq!(out, "a\nb");
    }

    #[test]
    fn a_border_name_that_is_not_one_is_refused() {
        assert_eq!(Border::parse("rounded"), Some(Border::Rounded));
        assert_eq!(Border::parse("double"), Some(Border::Double));
        assert_eq!(Border::parse("none"), Some(Border::None));
        assert_eq!(Border::parse("fancy"), None);
        assert_eq!(Border::parse(""), None);
    }

    /// An explicit width widens but never truncates: a box narrower than its text would cut the
    /// text, and losing the caller's data to make a border fit is never the right trade.
    #[test]
    fn an_explicit_width_widens_but_does_not_cut() {
        let wide = plain(&style(&Styling {
            text: "hi".to_string(),
            width: Some(10),
            border: Border::Square,
            ..Styling::default()
        }));
        assert_eq!(printed_width(wide.lines().next().unwrap()), 12);

        let narrow = plain(&style(&Styling {
            text: "much longer than four".to_string(),
            width: Some(4),
            ..Styling::default()
        }));
        assert!(narrow.contains("much longer than four"), "{narrow:?}");
    }
}
