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
static ENABLED: AtomicU8 = AtomicU8::new(0);
static MODE: AtomicU8 = AtomicU8::new(0);

pub fn set_enabled(on: bool) {
    ENABLED.store(u8::from(on), Ordering::Relaxed);
}

/// Whether vi mode is in force: **configured on, and not turned off at run time**.
///
/// Two questions rather than one. `ENABLED` is what `oslo.vi.enabled` asked for; the feature bit is
/// whether it applies right now. Because the config value is never overwritten, turning the feature
/// back on gives you back exactly what the config said — a shell configured for emacs does not
/// acquire vi mode by having the `vi` feature enabled.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) != 0 && oslo_base::feature::on(oslo_base::feature::at::VI)
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
        // Every key the editor's own vi keymap answers by setting insert mode, and no others.
        //
        // `c` and `s` belong here even though they take a motion, because the editor switches to
        // insert *the moment it sees the operator* and only then reads the motion. And it reads
        // that motion straight off the input, never through the key map — so the argument to
        // `cw`, `dw`, `fx` or `ra` is not a keystroke this ever sees. There is no key here that
        // could be somebody's argument, which is why this list needs no exceptions.
        (Mode::Normal, 'i' | 'I' | 'a' | 'A' | 'o' | 'O' | 'c' | 'C' | 's' | 'S') => Mode::Insert,
        (Mode::Normal, 'R') => Mode::Replace,
        _ => now,
    }
}

/// Forget everything remembered about the line that just ended.
///
/// The line editor starts every line in insert mode, but nothing told this module that — so after
/// leaving a line in normal mode the *next* prompt was drawn saying `N` while the editor was
/// already back in insert. That is the other half of the inconsistency, and the half that survived
/// until you typed something.
pub fn reset() {
    MODE.store(Mode::Insert.code(), Ordering::Relaxed);
}

/// Reset the mode and answer the cursor shape a fresh line starts with.
///
/// Called as a line is accepted, abandoned or ended. The shape has to go back because the next
/// line starts in insert: without it, a line left in normal mode leaves a block cursor sitting
/// over the one you type next. Empty when vi is off, which is when there is no shape to restore.
pub fn back_to_insert(vi_on: bool) -> String {
    reset();
    if !vi_on {
        return String::new();
    }
    let cursors = crate::settings::current().vi.cursors;
    cursors.for_mode(Mode::Insert).escape().to_string()
}

/// Record the mode, answering the cursor escape to write when it has changed.
///
/// `None` when nothing changed, so the common case — a keystroke that does not switch mode —
/// writes nothing at all. A cursor escape on every keypress would be harmless but is a great many
/// bytes to send for no reason.
pub fn observe(mode: Mode, cursors: &Cursors) -> Option<&'static str> {
    let previous = Mode::from_code(MODE.swap(mode.code(), Ordering::Relaxed));
    // The mode-change hooks, from the one place that already knows a change happened. `pre` and
    // `post` fire back to back here because the change *is* the swap above — there is no window
    // between them to do anything in, and a `pre` that fired before the swap would be reporting a
    // mode the editor had already left. Both exist so a handler can be written against either
    // name without having to know that.
    if previous != mode {
        // A prompt that shows the mode is now wrong. This is the only place that knows the instant
        // it changed, and the editor's redraw loop reads the counter rather than being told.
        crate::prompt::invalidate();
        let fields = [
            ("kind", "vi"),
            ("from", previous.name()),
            ("to", mode.name()),
        ];
        oslo_base::hooks::fire_at_here(oslo_base::hooks::at::PRE_MODE_CHANGE, &fields);
        oslo_base::hooks::fire_at_here(oslo_base::hooks::at::POST_MODE_CHANGE, &fields);
    }
    escape_for_change(previous, mode, cursors)
}

/// The escape a move from `previous` to `next` calls for, if any.
///
/// Separate from [`observe`] so the rule can be tested without touching the process-wide mode —
/// a test that flips those globals races every other test that reads them, and the prompt reads
/// them on every render.
fn escape_for_change(previous: Mode, next: Mode, cursors: &Cursors) -> Option<&'static str> {
    (previous != next).then(|| cursors.for_mode(next).escape())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys that start inserting are exactly the ones the editor's own keymap answers that
    /// way. `c` and `s` are in because the editor switches on the operator and reads the motion
    /// off the input afterwards, so the motion never reaches this.
    #[test]
    fn the_insert_starting_keys_match_the_editors_own() {
        for key in ['i', 'I', 'a', 'A', 'o', 'O', 'c', 'C', 's', 'S'] {
            assert_eq!(after_key(Mode::Normal, Some(key)), Mode::Insert, "{key}");
        }
        // An operator that does not insert, and a motion, both leave normal mode alone.
        for key in ['d', 'y', 'w', '0', 'x'] {
            assert_eq!(after_key(Mode::Normal, Some(key)), Mode::Normal, "{key}");
        }
        assert_eq!(after_key(Mode::Normal, Some('R')), Mode::Replace);
        assert_eq!(after_key(Mode::Insert, Some('\x1b')), Mode::Normal);
        assert_eq!(after_key(Mode::Replace, Some('\x1b')), Mode::Normal);
        // Esc in normal mode is already there.
        assert_eq!(after_key(Mode::Normal, Some('\x1b')), Mode::Normal);
        // A typed letter in insert mode is text, not a command.
        assert_eq!(after_key(Mode::Insert, Some('i')), Mode::Insert);
    }

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

        assert_eq!(
            escape_for_change(Mode::Insert, Mode::Insert, &cursors),
            None,
            "no change, no escape"
        );
        assert_eq!(
            escape_for_change(Mode::Insert, Mode::Normal, &cursors),
            Some("\x1b[2 q")
        );
        assert_eq!(
            escape_for_change(Mode::Normal, Mode::Insert, &cursors),
            Some("\x1b[6 q")
        );
        assert_eq!(
            escape_for_change(Mode::Normal, Mode::Replace, &cursors),
            Some("\x1b[4 q")
        );
    }

    /// The prompt needs a short name, as fish's `fish_mode_prompt` shows.
    #[test]
    fn each_mode_has_a_name_for_the_prompt() {
        assert_eq!(Mode::Normal.name(), "N");
        assert_eq!(Mode::Insert.name(), "I");
    }
}
