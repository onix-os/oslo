//! What the editor needs to know about the shell, and nothing else.
//!
//! # Why a trait rather than the `Environment` itself
//!
//! The line editor, completion and highlighting all want to ask the shell questions — what is
//! `$PS2`, what is on `$PATH`, which names are builtins, aliases or functions. Holding an
//! `Environment` to ask them is the obvious way and it is the wrong one: it points the interface
//! layer at the shell, and the shell is built on top of the interface layer. That is a cycle, and
//! it is the only thing standing between `ui` and being a crate of its own.
//!
//! Eight questions, none of which needs the store itself. The store answers them; so could a test
//! double, which is the other thing this buys — the editor's tests no longer have to build a whole
//! shell environment to ask what colour a word should be.
//!
//! # Why the collections are borrowed
//!
//! `aliases` and `functions` hand back references to the maps the shell already holds. Completion
//! walks them per keystroke, and copying two `HashMap`s to answer "what starts with `gi`" would be
//! work done on the typing path for nothing. `builtin_names` is the exception: it is a `Vec`
//! because an `impl Iterator` return is not object-safe, and this is called once per completion
//! rather than once per candidate.

use oslo_base::ast::Command;
use std::collections::HashMap;

/// The shell, as far as the editor is concerned.
pub trait Shell: Send {
    /// Whether this is a shell someone is typing at.
    ///
    /// Two things hang off it: the dropdown takes the terminal over, and the frecency table is
    /// read from and appended to a file in `$HOME`. `$-` is the signal rather than `isatty`,
    /// because `cargo test` inherits a terminal on stdin and a test must not write to the user's
    /// home directory.
    fn interactive(&self) -> bool;

    /// A shell variable's value.
    fn var(&self, name: &str) -> Option<&str>;

    /// Every variable and its value, for completing a `$name`.
    fn vars(&self) -> HashMap<String, String>;

    /// Whether `name` is a builtin — which decides how it is painted.
    fn is_builtin(&self, name: &str) -> bool;

    /// Every builtin's name, for completing a command word.
    fn builtin_names(&self) -> Vec<String>;

    /// What `name` is an alias for, if it is one.
    fn alias(&self, name: &str) -> Option<&str>;

    /// Every alias, for completion and for painting.
    fn aliases(&self) -> &HashMap<String, String>;

    /// Whether `name` is a shell function.
    fn is_function(&self, name: &str) -> bool;

    /// Every function, for completion and for painting.
    fn functions(&self) -> &HashMap<String, std::sync::Arc<Command>>;
}
