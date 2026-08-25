//! What a handler answered, on its way from Lua into the editor.
//!
//! Split from [`super`] because it is the shape the two sides agree on rather than part of the
//! state machine: `oslo-runtime` builds these out of a Lua table, and [`super::Session::apply`]
//! only ever reads them.

/// What a `key` hook asked the editor to do with the keystroke it just saw.
///
/// The third possibility — carry on as normal — is the `None` the hook answers with, so it needs
/// no variant here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyHook {
    /// Consume the keystroke: the editor never sees it, and the line is untouched.
    Swallow,
    /// Put this line in place instead.
    Line(Placed),
}

/// A line a handler asked for, with its cursor already in characters.
///
/// Named rather than a tuple because it is what both routes into the editor answer with — a key
/// the config bound and the `key` hook — and a fourth `bool` in a row is a thing to get wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub text: String,
    pub cursor: usize,
    /// Run it, as though Enter had been pressed.
    pub submit: bool,
    /// Run it without ever showing it. See [`crate::editor::Answer::erase`].
    pub erase: bool,
}
