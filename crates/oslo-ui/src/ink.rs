//! Colouring a string by naming the colour, the way `colored` does it.
//!
//! ```
//! use oslo_ui::ink::ink;
//! println!("{}", ink("done").green().bold());
//! println!("{}", ink(" WARN ").black().on_yellow());
//! println!("{}", ink("6.13 MB").rgb(0x61, 0xff, 0xca).underline());
//! ```
//!
//! # Why this is written here rather than depended on
//!
//! The [`colored`](https://docs.rs/colored) crate is the shape everybody knows, and the shape is
//! the valuable part — `ink(x).red().on_blue().bold()` needs no documentation. What a dependency
//! would bring with it is a second opinion about what "red" means, and a second answer to *should
//! this be coloured at all*, which is the only hard question in the subject. oslo answers that once,
//! in [`crate::theme::depth`], from `$NO_COLOR`, `$TERM` and `$COLORTERM`; a second crate deciding
//! it separately is how a `NO_COLOR` run comes out half-painted.
//!
//! So this is that surface over [`crate::theme::Style`], which oslo already had: the same builder, the same
//! names, one depth decision, no new crate.
//!
//! **Not `paint`** — [`crate::paint`] is the module that draws a block of rows under the cursor and
//! takes it away again, and two things called painting in one crate is one too many.
//!
//! # It resolves at `Display`, not at construction
//!
//! [`crate::ink::Inked`] carries the text and a [`crate::theme::Style`] and paints when it is *written*. The depth can change
//! after a value is built — a config that turns colour down, a pipe, `oslo.theme.depth("off")` — and
//! a string that had already baked its escapes in would be wrong by then. It also makes
//! [`plain`](crate::ink::Inked::plain) free: the text was never modified.

use crate::theme::{self, Color, Style};
use std::fmt;

/// Text with a style, painted when it is written out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inked {
    text: String,
    style: Style,
}

/// Start colouring `text`.
pub fn ink(text: impl Into<String>) -> Inked {
    Inked {
        text: text.into(),
        style: Style::default(),
    }
}

/// The eight basic colours as foreground and background methods, in one place.
///
/// A macro because the alternative is thirty-two functions differing by one token each, which is
/// thirty-two chances to write `Blue` in the body of `fn green`.
macro_rules! basics {
    ($($name:ident, $bright:ident, $on:ident, $on_bright:ident => $index:expr;)*) => {
        impl Inked {
            $(
                pub fn $name(self) -> Inked { self.fg(Color::Basic { index: $index, bright: false }) }
                pub fn $bright(self) -> Inked { self.fg(Color::Basic { index: $index, bright: true }) }
                pub fn $on(self) -> Inked { self.on(Color::Basic { index: $index, bright: false }) }
                pub fn $on_bright(self) -> Inked { self.on(Color::Basic { index: $index, bright: true }) }
            )*
        }
    };
}

basics! {
    black,   bright_black,   on_black,   on_bright_black   => 0;
    red,     bright_red,     on_red,     on_bright_red     => 1;
    green,   bright_green,   on_green,   on_bright_green   => 2;
    yellow,  bright_yellow,  on_yellow,  on_bright_yellow  => 3;
    blue,    bright_blue,    on_blue,    on_bright_blue    => 4;
    magenta, bright_magenta, on_magenta, on_bright_magenta => 5;
    cyan,    bright_cyan,    on_cyan,    on_bright_cyan    => 6;
    white,   bright_white,   on_white,   on_bright_white   => 7;
}

impl Inked {
    /// A foreground colour of any kind.
    pub fn fg(mut self, colour: Color) -> Inked {
        self.style.fg = Some(colour);
        self
    }

    /// A background colour of any kind. `on` rather than `bg`, which is `colored`'s word and reads
    /// as English at the call site: `ink(" ok ").black().on(green)`.
    pub fn on(mut self, colour: Color) -> Inked {
        self.style.bg = Some(colour);
        self
    }

    /// A palette index, 0..=255.
    pub fn indexed(self, index: u8) -> Inked {
        self.fg(Color::Indexed(index))
    }

    pub fn on_indexed(self, index: u8) -> Inked {
        self.on(Color::Indexed(index))
    }

    /// 24-bit colour, downgraded to the nearest palette entry on a terminal that cannot show it
    /// rather than dropped. See `Color::sgr`.
    pub fn rgb(self, r: u8, g: u8, b: u8) -> Inked {
        self.fg(Color::Rgb(r, g, b))
    }

    pub fn on_rgb(self, r: u8, g: u8, b: u8) -> Inked {
        self.on(Color::Rgb(r, g, b))
    }

    /// A colour written the way a theme writes one: `"green"`, `"brightblue"`, `"#61ffca"`, `"240"`.
    ///
    /// An unreadable name leaves the colour alone rather than guessing, which is what
    /// [`Color::parse`] promises and the reason it answers `Option`.
    pub fn named(self, name: &str) -> Inked {
        match Color::parse(name) {
            Some(colour) => self.fg(colour),
            None => self,
        }
    }

    pub fn on_named(self, name: &str) -> Inked {
        match Color::parse(name) {
            Some(colour) => self.on(colour),
            None => self,
        }
    }

    pub fn bold(mut self) -> Inked {
        self.style.bold = true;
        self
    }

    /// `dim`, not `dimmed`: the [`Style`] field is `dim`, and a builder that renamed it would make
    /// the two spellings a thing to remember.
    pub fn dim(mut self) -> Inked {
        self.style.dim = true;
        self
    }

    pub fn italic(mut self) -> Inked {
        self.style.italic = true;
        self
    }

    pub fn underline(mut self) -> Inked {
        self.style.underline = true;
        self
    }

    pub fn reverse(mut self) -> Inked {
        self.style.reverse = true;
        self
    }

    pub fn blink(mut self) -> Inked {
        self.style.blink = true;
        self
    }

    pub fn hidden(mut self) -> Inked {
        self.style.hidden = true;
        self
    }

    pub fn strike(mut self) -> Inked {
        self.style.strike = true;
        self
    }

    /// Everything at once, for a caller that already holds a [`Style`] — a theme entry, usually.
    pub fn styled(mut self, style: Style) -> Inked {
        self.style = style;
        self
    }

    /// The style as it stands, for a caller that wants to paint something else the same way.
    pub fn style(&self) -> Style {
        self.style
    }

    /// The text with no escapes at all, whatever the terminal can do.
    pub fn plain(&self) -> &str {
        &self.text
    }

    /// Painted at a depth the caller names, for a test or a recording that must not depend on the
    /// terminal it happens to run in.
    pub fn at(&self, depth: theme::Depth) -> String {
        self.style.paint(&self.text, depth)
    }
}

/// Painted at whatever this terminal can do, decided once by [`theme::depth`].
impl fmt::Display for Inked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.at(theme::depth()))
    }
}

#[cfg(test)]
#[path = "ink/tests.rs"]
mod tests;
