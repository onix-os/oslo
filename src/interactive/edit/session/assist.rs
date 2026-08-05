//! What the shell plugs into an editing session.
//!
//! Split out of the state machine because it is a different kind of thing: [`super::Session`] is
//! logic with no dependencies, and this is the seam the shell reaches through. Everything
//! oslo-specific — highlighting, ghost hints, the completion dropdown, history, the Lua hooks —
//! arrives here, which is what lets the state machine be tested with [`NoAssist`] and nothing else.

use super::{Bound, Key, KeyHook};

/// What the shell supplies to an editing session.
///
/// Every method has a default that does nothing, so a test — or an early integration — can
/// implement none of them and still get a working editor.
pub trait Assist {
    /// The line as it should be drawn. Must print the same *characters*: the layout measures the
    /// plain text and draws this, so adding or removing anything but escapes moves the cursor.
    fn highlight(&mut self, line: &str) -> String {
        line.to_string()
    }

    /// Ghost text shown after the cursor, already styled.
    fn hint(&mut self, _line: &str, _cursor: usize) -> Option<String> {
        None
    }

    /// The same suggestion **without** styling, for accepting it into the line.
    ///
    /// Separate from [`Assist::hint`] because that one is painted, and inserting escapes into the
    /// command would put them in the history and in what runs.
    fn hint_text(&mut self, _line: &str, _cursor: usize) -> Option<String> {
        None
    }

    /// Run completion, answering the line and cursor it produced.
    ///
    /// The whole interaction belongs to the implementation — oslo's dropdown draws itself and
    /// takes its own keys — because a menu is a different mode, not a keystroke.
    fn complete(&mut self, _line: &str, _cursor: usize, _back: bool) -> Option<(String, usize)> {
        None
    }

    /// The previous history entry, given what is on the line now.
    fn history_prev(&mut self, _line: &str) -> Option<String> {
        None
    }

    fn history_next(&mut self) -> Option<String> {
        None
    }

    /// Ctrl-R. Answers a whole line to put in place, or `None` to leave things alone.
    fn search_history(&mut self, _line: &str) -> Option<String> {
        None
    }

    /// The space that ends a word has been typed: expand an abbreviation if this is one.
    ///
    /// Answers the line **including the space**, because the expansion and the space are one act —
    /// `gco ` becomes `git checkout ` in a single step, so what you see is a finished command
    /// rather than a word waiting to be finished.
    fn abbreviation(&mut self, _line: &str, _cursor: usize) -> Option<(String, usize)> {
        None
    }

    /// A key the config bound to a Lua handler. Answers the line the handler asked for.
    ///
    /// The name is oslo's spelling — `ctrl-s`, `alt-u`, `shift-tab` — so a config's key table can
    /// be looked up directly.
    /// The line the handler asked for, its cursor, and whether to run it.
    fn lua_key(
        &mut self,
        _name: &str,
        _line: &str,
        _cursor: usize,
    ) -> Option<(String, usize, bool)> {
        None
    }

    /// oslo's name for a key, when the config could have bound it. `None` means never ask.
    fn key_name(&mut self, _key: Key) -> Option<String> {
        None
    }

    /// What the config bound this key to, if anything.
    fn binding(&mut self, _key: Key) -> Option<Bound> {
        None
    }

    /// Whether anything is attached to the `key` hook.
    ///
    /// **Asked before the line is built**, and that is the entire reason it is a separate method.
    /// [`Assist::key_hook`] needs the text and the cursor, and producing the text means collecting
    /// the buffer into a `String`. A session with no `key` handler must not pay for that on every
    /// keystroke, so the question "is anyone listening" is answered first and cheaply — oslo's
    /// implementation is one atomic load.
    fn watches_keys(&mut self) -> bool {
        false
    }

    /// The `key` hook: a Lua handler that sees every keystroke before anything else does.
    ///
    /// `None` means the handler declined — or that there was none — and the key goes on to do
    /// whatever it would have done.
    fn key_hook(&mut self, _key: Key, _line: &str, _cursor: usize) -> Option<KeyHook> {
        None
    }
}

/// An `Assist` that does nothing, for tests and for a shell that has not wired one yet.
#[derive(Debug, Default)]
pub struct NoAssist;
impl Assist for NoAssist {}
