//! Vi mode, on fish's model.
//!
//! fish gets three things right that a bare `set -o vi` does not, and they are what make vi mode
//! usable rather than merely present:
//!
//! * **The cursor says which mode you are in.** A block in normal mode, a bar in insert. Without
//!   it the only way to know is to press a key and see what happens, which is how you end up with
//!   `dd` in the middle of a command.
//! * **The mode is available to the prompt**, so a `[N]`/`[I]` indicator is possible for terminals
//!   that ignore cursor-shape escapes.
//! * **The shapes are configurable**, because `block`, `line` and `underscore` are not equally
//!   visible on every colour scheme.
//!
//! oslo follows the same three, with `oslo.vi.cursor_*` where fish has `fish_cursor_*`.

use std::sync::atomic::{AtomicU8, Ordering};

/// Which vi mode the editor is in.
///
/// `Insert` is the starting mode, as in fish — a shell prompt is for typing, and starting in
/// normal mode means every line begins with an `i` nobody wanted to press.
/// There is no `Visual` here on purpose: the line editor's vi keymap has Command, Insert and
/// Replace and nothing else, so a visual mode would be a setting that could never take effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Normal,
    /// Overwrite — `R`.
    Replace,
}

impl Mode {
    fn code(self) -> u8 {
        match self {
            Mode::Insert => 0,
            Mode::Normal => 1,
            Mode::Replace => 3,
        }
    }

    fn from_code(code: u8) -> Mode {
        match code {
            1 => Mode::Normal,
            3 => Mode::Replace,
            _ => Mode::Insert,
        }
    }

    /// The short name a prompt shows, as fish's `fish_mode_prompt` does.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Insert => "I",
            Mode::Normal => "N",
            Mode::Replace => "R",
        }
    }
}

/// The cursor a terminal draws, as `DECSCUSR` spells it.
///
/// fish's three shapes and its `blink` suffix. The numbers are the escape's own: odd is blinking,
/// even is steady, which is why each pair differs by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    BlinkBlock,
    Block,
    BlinkUnderline,
    Underline,
    BlinkBar,
    Bar,
}

impl Cursor {
    /// The `CSI n SP q` this shape asks for.
    pub fn escape(self) -> &'static str {
        match self {
            Cursor::BlinkBlock => "\x1b[1 q",
            Cursor::Block => "\x1b[2 q",
            Cursor::BlinkUnderline => "\x1b[3 q",
            Cursor::Underline => "\x1b[4 q",
            Cursor::BlinkBar => "\x1b[5 q",
            Cursor::Bar => "\x1b[6 q",
        }
    }

    /// Parse the spelling a config uses: `block`, `line blink`, `underscore`.
    ///
    /// fish's vocabulary, including its `line` for what `DECSCUSR` calls a bar and `underscore`
    /// for what it calls an underline — a config written for fish should not have to be
    /// translated word by word.
    pub fn parse(text: &str) -> Option<Cursor> {
        let lower = text.trim().to_ascii_lowercase();
        let blink = lower.split_whitespace().any(|w| w == "blink");
        let shape = lower.split_whitespace().next()?;
        Some(match (shape, blink) {
            ("block", false) => Cursor::Block,
            ("block", true) => Cursor::BlinkBlock,
            ("underscore" | "underline", false) => Cursor::Underline,
            ("underscore" | "underline", true) => Cursor::BlinkUnderline,
            ("line" | "bar", false) => Cursor::Bar,
            ("line" | "bar", true) => Cursor::BlinkBar,
            _ => return None,
        })
    }
}

/// The cursor shape for each mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursors {
    pub insert: Cursor,
    pub normal: Cursor,
    pub replace: Cursor,
}

impl Default for Cursors {
    fn default() -> Self {
        // fish's defaults: a bar where you insert, a block where you command. The block sits *on*
        // a character, which is exactly what normal mode acts on.
        Cursors {
            insert: Cursor::Bar,
            normal: Cursor::Block,
            replace: Cursor::Underline,
        }
    }
}

impl Cursors {
    pub fn for_mode(&self, mode: Mode) -> Cursor {
        match mode {
            Mode::Insert => self.insert,
            Mode::Normal => self.normal,
            Mode::Replace => self.replace,
        }
    }
}

/// Whether vi mode is on, and which mode the editor is in.
///
/// Process-global rather than threaded through: the prompt, the highlighter and the key handler
/// all need it, and they are reached from three different places in rustyline's callbacks.
static ENABLED: AtomicU8 = AtomicU8::new(0);
static MODE: AtomicU8 = AtomicU8::new(0);

pub fn set_enabled(on: bool) {
    ENABLED.store(u8::from(on), Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) != 0
}

/// The mode the editor is in, or `None` when vi mode is off.
pub fn mode() -> Option<Mode> {
    enabled().then(|| Mode::from_code(MODE.load(Ordering::Relaxed)))
}

/// The mode a keystroke leads to, given the mode it was pressed in.
///
/// The line editor reports its mode *before* applying the key, so a handler watching keystrokes is
/// always one behind — Esc would change nothing until the next keypress, which is precisely when a
/// mode indicator stops being worth having. This closes that gap.
///
/// Deliberately not the vi keymap: only the keys that *enter* a mode are listed. Anything else
/// leaves the mode as it was, and a wrong guess costs nothing lasting, because the next keystroke
/// reads the editor's real mode again.
pub fn after_key(now: Mode, key: Option<char>) -> Mode {
    let Some(key) = key else {
        return now;
    };
    match (now, key) {
        // Esc leaves insert or replace for normal.
        (Mode::Insert | Mode::Replace, '\x1b') => Mode::Normal,
        // The keys that start inserting. `c` and `s` take an object first, so they are not here:
        // guessing insert on `c` would flicker on every `cw`.
        (Mode::Normal, 'i' | 'I' | 'a' | 'A' | 'o' | 'O' | 'C' | 'S') => Mode::Insert,
        (Mode::Normal, 'R') => Mode::Replace,
        _ => now,
    }
}

/// Record the mode, answering the cursor escape to write when it has changed.
///
/// `None` when nothing changed, so the common case — a keystroke that does not switch mode —
/// writes nothing at all. A cursor escape on every keypress would be harmless but is a great many
/// bytes to send for no reason.
pub fn observe(mode: Mode, cursors: &Cursors) -> Option<&'static str> {
    let previous = MODE.swap(mode.code(), Ordering::Relaxed);
    (previous != mode.code()).then(|| cursors.for_mode(mode).escape())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// fish's vocabulary, so a config written for fish reads the same here.
    #[test]
    fn the_fish_cursor_names_are_understood() {
        assert_eq!(Cursor::parse("block"), Some(Cursor::Block));
        assert_eq!(Cursor::parse("block blink"), Some(Cursor::BlinkBlock));
        assert_eq!(Cursor::parse("line"), Some(Cursor::Bar));
        assert_eq!(Cursor::parse("underscore"), Some(Cursor::Underline));
        // Spelled the way `DECSCUSR` does, too.
        assert_eq!(Cursor::parse("bar"), Some(Cursor::Bar));
        assert_eq!(
            Cursor::parse("underline blink"),
            Some(Cursor::BlinkUnderline)
        );
        assert_eq!(Cursor::parse("  BLOCK  "), Some(Cursor::Block));
        assert_eq!(Cursor::parse("wobbly"), None);
    }

    /// Odd is blinking, even is steady — the escape's own numbering.
    #[test]
    fn a_shape_asks_for_the_right_escape() {
        assert_eq!(Cursor::Block.escape(), "\x1b[2 q");
        assert_eq!(Cursor::BlinkBlock.escape(), "\x1b[1 q");
        assert_eq!(Cursor::Bar.escape(), "\x1b[6 q");
    }

    /// Only a *change* writes an escape: a keystroke that stays in the same mode sends nothing.
    #[test]
    fn the_cursor_is_only_redrawn_when_the_mode_changes() {
        let cursors = Cursors::default();
        set_enabled(true);
        MODE.store(Mode::Insert.code(), Ordering::Relaxed);

        assert_eq!(
            observe(Mode::Insert, &cursors),
            None,
            "no change, no escape"
        );
        assert_eq!(observe(Mode::Normal, &cursors), Some("\x1b[2 q"));
        assert_eq!(observe(Mode::Normal, &cursors), None);
        assert_eq!(observe(Mode::Insert, &cursors), Some("\x1b[6 q"));
        assert_eq!(mode(), Some(Mode::Insert));

        set_enabled(false);
        assert_eq!(mode(), None, "off means there is no mode to report");
    }

    /// The prompt needs a short name, as fish's `fish_mode_prompt` shows.
    #[test]
    fn each_mode_has_a_name_for_the_prompt() {
        assert_eq!(Mode::Normal.name(), "N");
        assert_eq!(Mode::Insert.name(), "I");
    }
}
