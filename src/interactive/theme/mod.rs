//! The colours the prompt draws with, and where they come from.
//!
//! One [`Theme`], read from `oslo.theme` in the config, used by three places that previously each
//! carried their own hardcoded escapes: the dropdown, the syntax highlighter and the prompt.
//!
//! **Merged, not replaced.** A config that writes `oslo.theme = { syntax = { command = "cyan" } }`
//! means "make commands cyan", not "discard every other colour". A whole-table assignment is the
//! natural way to write one field and it must not silently blank the other forty — so every field
//! is an `Option` layered over [`Theme::default`], and only what the config names is overridden.

pub mod color;
mod from_lua;

pub use color::{Color, Depth, Style};
pub use from_lua::read_lua_theme;

use std::sync::RwLock;

/// The theme in force.
///
/// Process-wide rather than threaded through the helper, because the three consumers are reached
/// from rustyline callbacks that own no state of ours. Written once when the config loads and read
/// on every keystroke, which is what `RwLock` is for.
static THEME: RwLock<Option<Theme>> = RwLock::new(None);

/// The colour depth in force, decided once.
static DEPTH: RwLock<Option<Depth>> = RwLock::new(None);

/// Read the theme. Cheap enough for a keystroke: a read lock and a clone of a plain struct.
pub fn current() -> Theme {
    THEME
        .read()
        .ok()
        .and_then(|t| t.clone())
        .unwrap_or_default()
}

/// Install a theme, replacing whatever was there.
pub fn install(theme: Theme) {
    if let Ok(mut slot) = THEME.write() {
        *slot = Some(theme);
    }
}

/// The colour depth, detected on first use.
///
/// Cached because `$TERM` and `$COLORTERM` do not change during a session, and because this is
/// read once per styled span — several hundred times for one redraw of a full dropdown.
pub fn depth() -> Depth {
    if let Ok(slot) = DEPTH.read()
        && let Some(depth) = *slot
    {
        return depth;
    }
    let detected = Depth::detect();
    if let Ok(mut slot) = DEPTH.write() {
        *slot = Some(detected);
    }
    detected
}

/// Force a depth, for tests and for a config that knows better than the environment.
pub fn set_depth(depth: Depth) {
    if let Ok(mut slot) = DEPTH.write() {
        *slot = Some(depth);
    }
}

/// Everything the interactive layer draws with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Theme {
    pub syntax: Syntax,
    pub pager: Pager,
    pub prompt: Prompt,
}

/// Colours for the line as it is typed.
///
/// The names are fish's, because they are the ones people already have in a config somewhere and
/// because the set is a good specification of how deep highlighting should go. `builtin`,
/// `function` and `keyword` fall back to `command` when a theme leaves them out, which is what
/// keeps a two-line theme from looking half-finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Syntax {
    pub command: Style,
    pub builtin: Style,
    pub function: Style,
    pub keyword: Style,
    /// A command name that resolves to nothing. fish's most useful colour.
    pub error: Style,
    /// A command that runs what follows it as another user: `sudo`, `doas`, `su`.
    pub danger: Style,
    pub param: Style,
    /// A parameter that names a file which exists.
    pub valid_path: Style,
    pub option: Style,
    /// A glob metacharacter: `*`, `?`, `[…]`.
    pub glob: Style,
    /// A bare number.
    pub number: Style,
    /// The `NAME=` of an assignment.
    pub assignment: Style,
    /// `'…'` — literal throughout.
    pub single_quote: Style,
    /// The literal parts of `"…"`. What expands inside it takes the variable colour instead.
    pub double_quote: Style,
    pub escape: Style,
    pub operator: Style,
    pub redirection: Style,
    /// `;` and `&`.
    pub end: Style,
    pub comment: Style,
    pub variable: Style,
    pub autosuggestion: Style,
    pub match_bracket: Style,
}

impl Default for Syntax {
    fn default() -> Self {
        let rgb = |r: u8, g: u8, b: u8| Style::fg(Color::Rgb(r, g, b));
        Syntax {
            // **RGB, not the sixteen ANSI slots.** A palette tool like pywal remaps what
            // `basic(2)` means, so a theme built on the slots changes colour whenever the wallpaper
            // does. These are absolute: red stays red. Only the *syntax* palette is pinned this
            // way — the prompt and pager deliberately keep the slots, so they still follow the
            // terminal's scheme.
            command: rgb(0x50, 0xfa, 0x7b),
            builtin: Style {
                bold: true,
                ..rgb(0x50, 0xfa, 0x7b)
            },
            function: rgb(0x8b, 0xe9, 0xfd),
            keyword: rgb(0xff, 0x79, 0xc6),
            error: Style {
                underline: true,
                ..rgb(0xff, 0x55, 0x55)
            },
            // A command that runs everything after it as another user. Black on red, because it
            // is the one word in a line whose presence changes what every other word can do.
            danger: Style {
                bold: true,
                bg: Some(Color::Rgb(0xff, 0x55, 0x55)),
                ..rgb(0x00, 0x00, 0x00)
            },
            param: Style::default(),
            // Underline rather than a colour: it has to compose with whatever colour the
            // parameter already has, and a second colour would fight with it.
            valid_path: Style {
                underline: true,
                ..Style::default()
            },
            option: rgb(0xff, 0xb8, 0x6c),
            // A glob is the one thing in a line that can turn one word into fifty, and it should
            // be impossible to miss.
            glob: Style {
                bold: true,
                ..rgb(0xff, 0x79, 0xc6)
            },
            number: rgb(0xbd, 0x93, 0xf9),
            assignment: rgb(0x50, 0xfa, 0x7b),
            // Two yellows: a single-quoted string is inert and takes the plainer one, a
            // double-quoted one still expands and is brighter to say so.
            single_quote: rgb(0xd8, 0xdf, 0x6e),
            double_quote: rgb(0xf1, 0xfa, 0x8c),
            escape: rgb(0xff, 0x79, 0xc6),
            operator: rgb(0x8b, 0xe9, 0xfd),
            redirection: rgb(0xff, 0xb8, 0x6c),
            end: rgb(0x62, 0x72, 0xa4),
            comment: rgb(0x62, 0x72, 0xa4),
            variable: rgb(0xbd, 0x93, 0xf9),
            // Colour 238, an explicit index rather than the bright-black slot: a ghost has to
            // sit *behind* the text you are typing, and asking for a specific grey is the only way
            // to say how far behind. It degrades to bright black on a sixteen-colour terminal,
            // which keeps it dim there too.
            autosuggestion: Style::fg(Color::Indexed(238)),
            match_bracket: Style {
                bold: true,
                ..Style::default()
            },
        }
    }
}

/// Colours for the completion dropdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pager {
    /// The background every unselected row is drawn on.
    ///
    /// This is what makes the menu read as a block rather than as loose text: there is no border
    /// and no caption, so the colour is the only thing saying where it starts and stops.
    pub bg: Option<Color>,
    pub text: Style,
    pub text_sel: Style,
    pub sel_bg: Option<Color>,
    /// The background a kind badge takes **on the selected row only**.
    ///
    /// One colour for whichever kind is selected, rather than each kind keeping its own: the
    /// selected row is already marked out by `sel_bg`, and a badge that kept its usual colour
    /// there reads as a second, competing highlight. This is the same job IRIS does by inverting
    /// the pill — a different treatment for the row you are on.
    pub kind_sel: Option<Color>,
    /// The part of a candidate the user has already typed.
    pub match_: Style,
    pub desc: Style,
    pub desc_sel: Style,
    /// Every info column after the description: a file's size, a directory's entry count, what an
    /// alias expands to, and anything `oslo.completion.columns` adds.
    ///
    /// Dimmer than the description on purpose. These columns annotate a candidate where the
    /// description explains it, and a row with four equally loud columns is a row nothing stands
    /// out in — the label is what the eye is looking for.
    pub extra: Style,
    pub extra_sel: Style,
    pub scroll: Style,
    /// The pill, one entry per completion kind.
    pub kind: KindColors,
}

impl Pager {
    /// The style for info column `col`. Column 0 is the description; the rest share one style,
    /// because a theme that had to name a colour per column would break the moment a config added
    /// one more.
    pub fn column(&self, col: usize, selected: bool) -> Style {
        match (col, selected) {
            (0, false) => self.desc,
            (0, true) => self.desc_sel,
            (_, false) => self.extra,
            (_, true) => self.extra_sel,
        }
    }
}

impl Default for Pager {
    fn default() -> Self {
        Pager {
            bg: Some(Color::Indexed(236)),
            text: Style::default(),
            text_sel: Style {
                bold: true,
                ..Style::fg(Color::Basic {
                    index: 7,
                    bright: true,
                })
            },
            sel_bg: Some(Color::Indexed(238)),
            kind_sel: Some(Color::Indexed(242)),
            match_: Style {
                bold: true,
                ..Style::fg(Color::Basic {
                    index: 6,
                    bright: true,
                })
            },
            desc: Style::fg(Color::Indexed(245)),
            desc_sel: Style::fg(Color::Indexed(252)),
            extra: Style::fg(Color::Indexed(242)),
            extra_sel: Style::fg(Color::Indexed(248)),
            scroll: Style::fg(Color::Indexed(240)),
            kind: KindColors::default(),
        }
    }
}

/// The pill colours, by completion kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindColors {
    pub command: Style,
    pub builtin: Style,
    pub file: Style,
    pub dir: Style,
    pub variable: Style,
    pub history: Style,
    pub alias: Style,
    /// Anything a theme has no entry for.
    pub other: Style,
}

impl Default for KindColors {
    fn default() -> Self {
        // Dark text on a coloured field, which is what makes a pill read as a pill rather than as
        // coloured words. Indexed rather than 24-bit so the default theme looks the same on a
        // 256-colour terminal as on a modern one.
        let pill = |bg: u8| Style {
            fg: Some(Color::Indexed(233)),
            bg: Some(Color::Indexed(bg)),
            ..Style::default()
        };
        KindColors {
            command: pill(140),
            builtin: pill(79),
            file: pill(245),
            dir: pill(240),
            variable: pill(215),
            history: pill(79),
            alias: pill(140),
            other: pill(245),
        }
    }
}

impl KindColors {
    /// The pill for a `CompletionCandidate::kind`.
    pub fn for_kind(&self, kind: &str) -> Style {
        match kind {
            "command" => self.command,
            "builtin" => self.builtin,
            "file" => self.file,
            "dir" | "directory" => self.dir,
            "variable" => self.variable,
            "history" => self.history,
            "alias" => self.alias,
            _ => self.other,
        }
    }
}

/// Colours the built-in prompt draws with, when no Lua prompt has replaced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub cwd: Style,
    pub host: Style,
    pub user: Style,
    pub git: Style,
    pub ok: Style,
    pub failed: Style,
    /// The clock and the duration in the right prompt. Dim on purpose: they are there to be
    /// glanced at, not read, and a right prompt that competes with the command is a nuisance.
    pub aside: Style,
}

impl Default for Prompt {
    fn default() -> Self {
        let basic = |index: u8| {
            Style::fg(Color::Basic {
                index,
                bright: false,
            })
        };
        Prompt {
            cwd: Style {
                bold: true,
                ..basic(4)
            },
            host: Style {
                bold: true,
                ..basic(5)
            },
            user: basic(6),
            git: basic(2),
            ok: Style {
                bold: true,
                ..basic(2)
            },
            failed: Style {
                bold: true,
                ..basic(1)
            },
            aside: Style {
                dim: true,
                ..basic(7)
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_theme_paints_something_for_every_role() {
        let theme = Theme::default();
        set_depth(Depth::Ansi256);
        // Not exhaustive by field name — the point is that no role is left silently plain except
        // the ones that are meant to be.
        assert!(!theme.syntax.command.is_plain());
        assert!(!theme.syntax.error.is_plain());
        assert!(theme.pager.bg.is_some());
        assert!(!theme.pager.kind.command.is_plain());
        // `param` is deliberately plain: an ordinary argument takes the terminal's own colour.
        assert!(theme.syntax.param.is_plain());
    }

    #[test]
    fn an_unknown_kind_still_gets_a_pill() {
        let kinds = KindColors::default();
        assert_eq!(kinds.for_kind("nonesuch"), kinds.other);
        assert_eq!(kinds.for_kind("dir"), kinds.dir);
        // Both spellings, since the completer says `dir` and a Lua theme may say `directory`.
        assert_eq!(kinds.for_kind("directory"), kinds.dir);
    }

    #[test]
    fn installing_a_theme_replaces_the_current_one() {
        let mut theme = Theme::default();
        theme.syntax.command = Style::fg(Color::Indexed(99));
        install(theme.clone());
        assert_eq!(current().syntax.command, Style::fg(Color::Indexed(99)));
        install(Theme::default());
    }
}
