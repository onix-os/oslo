//! What a key was bound to, and what pressing it did.
//!
//! Two enums either side of the session: [`Bound`] is what the keymap resolved a chord *to*, and
//! [`Step`] is what the session did *about it*. They are separate because most bindings are handled
//! where they are read and a few cannot be — opening a widget needs the terminal, which belongs to
//! the loop outside — so `Step` is how the session says "this one is yours".

/// A binding the config asked for, which the session performs instead of its default.
///
/// Named for the effect rather than the key, because the same effect can be reached from a chord,
/// a config entry or a default — and the loop performing it should not care which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    /// Switch the prompt between shell and Lua.
    ToggleLanguage,
    ClearScreen,
    SearchHistory,
    /// Take the whole ghost suggestion.
    AcceptHint,
    /// Take one word of it.
    AcceptHintWord,
    Interrupt,
    Complete,
    /// Open the tab finder. Like completion, it wants the terminal to itself, so the session only
    /// says so and the outer loop does it.
    OpenScratch,
    /// Open the macro manager, on the same terms.
    OpenMacros,
    /// A Lua function, by the key's name.
    Lua(String),
}

/// What a keypress did to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Keep editing. `redraw` is false for a key that changed nothing, so an unbound chord does
    /// not repaint the row.
    Continue { redraw: bool },
    /// Enter. `erase` runs the line without ever showing it — see [`crate::editor::Answer::erase`].
    Accept { erase: bool },
    /// Ctrl-C.
    Interrupted,
    /// Ctrl-D on an empty line.
    Eof,
    /// Ctrl-L: the screen should be cleared before the next draw.
    ClearScreen,
    /// Shift-Tab: the prompt should switch between shell and Lua. Handled by the read loop, which
    /// is the only thing that knows what a language *is*.
    ToggleLanguage,
    /// Open the completion modal through the outer loop's shared input reader.
    OpenCompletion { backwards: bool },
    /// Open the tab finder through the outer loop, which owns the terminal the widget needs.
    OpenScratch,
    /// Open the macro manager, likewise.
    OpenMacros,
}
