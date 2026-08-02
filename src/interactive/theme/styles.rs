//! Named styles a config defines, so a prompt says *what* a piece is and the theme says how it looks.
//!
//! ```lua
//! oslo.theme.styles["git.branch"] = "fg:cyan bold"
//! oslo.theme.styles["prompt.error"] = "bg:1 fg:0"
//! ```
//!
//! A segment then writes `style = "git.branch"` and never mentions a colour. That indirection is
//! the whole point of naming styles: a colour scheme is one table, and changing it does not mean
//! finding every place a colour was written down.
//!
//! The spelling is hexe's — space-separated words, `fg:` and `bg:` for colours and bare words for
//! attributes — so the same string works in both.

use super::{Color, Style};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn table() -> &'static Mutex<HashMap<String, Style>> {
    static STYLES: OnceLock<Mutex<HashMap<String, Style>>> = OnceLock::new();
    STYLES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Define a named style, replacing any earlier definition.
pub fn define(name: &str, spec: &str) {
    if let Ok(mut t) = table().lock() {
        t.insert(name.to_string(), parse(spec));
    }
}

/// Look up a named style, or `None` if the config never defined one.
pub fn lookup(name: &str) -> Option<Style> {
    table().lock().ok()?.get(name).copied()
}

/// Forget every definition — used when a config is reloaded, so a name it no longer defines stops
/// resolving to what it used to mean.
pub fn clear() {
    if let Ok(mut t) = table().lock() {
        t.clear();
    }
}

/// Read hexe's style spelling: `fg:cyan bold`, `bg:1 fg:0`, `#8be9fd underline`.
///
/// A bare colour with no `fg:` is a foreground, because that is what someone writing `"cyan"`
/// means, and requiring the prefix would make the common case the wordy one.
pub fn parse(spec: &str) -> Style {
    let mut style = Style::default();
    for word in spec.split_whitespace() {
        match word {
            "bold" => style.bold = true,
            "dim" => style.dim = true,
            "italic" => style.italic = true,
            "underline" | "underlined" => style.underline = true,
            "reverse" => style.reverse = true,
            _ => {
                if let Some(colour) = word.strip_prefix("fg:") {
                    style.fg = Color::parse(colour);
                } else if let Some(colour) = word.strip_prefix("bg:") {
                    style.bg = Color::parse(colour);
                } else if let Some(colour) = Color::parse(word) {
                    style.fg = Some(colour);
                }
                // An unreadable word is skipped rather than refused: a style string is written by
                // hand and a typo in one attribute should not blank the whole prompt piece.
            }
        }
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    /// hexe's spelling, understood as written.
    #[test]
    fn a_style_string_reads_the_way_hexe_writes_it() {
        let s = parse("bg:1 fg:0");
        assert_eq!(s.bg, Color::parse("1"));
        assert_eq!(s.fg, Color::parse("0"));

        let s = parse("fg:cyan bold");
        assert_eq!(s.fg, Color::parse("cyan"));
        assert!(s.bold);
        assert!(!s.dim);
    }

    /// A bare colour is a foreground, because that is what writing one means.
    #[test]
    fn a_bare_colour_is_the_foreground() {
        let s = parse("#8be9fd underline");
        assert_eq!(s.fg, Color::parse("#8be9fd"));
        assert!(s.underline);
    }

    /// A typo in one word costs that word, not the whole style.
    #[test]
    fn an_unreadable_word_is_skipped() {
        let s = parse("bold notacolour fg:red");
        assert!(s.bold);
        assert_eq!(s.fg, Color::parse("red"));
    }

    /// Defining a name makes it resolvable; clearing forgets it.
    #[test]
    fn names_resolve_until_they_are_cleared() {
        define("test.only.branch", "fg:green bold");
        let found = lookup("test.only.branch").expect("just defined");
        assert!(found.bold);
        clear();
        assert!(lookup("test.only.branch").is_none());
    }
}
