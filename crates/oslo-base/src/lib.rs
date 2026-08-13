//! The bottom of oslo: what everything above it is built out of.
//!
//! Five modules with one thing in common — none of them knows there is a shell above them. That is
//! the whole selection rule, and it was measured rather than guessed: every one of these had zero
//! references to `ui`, `exec`, `expand` or the syntax layer before it was moved, and the compiler
//! now keeps it that way.
//!
//! * [`ast`] — the syntax tree the parser produces and the executor walks.
//! * [`error`] — the one error type the evaluator unwinds with.
//! * [`feature`] — the parts of the shell a config can turn off and on again while it runs.
//! * [`hooks`] — where the shell reaches a hook, without knowing that Lua exists.
//! * [`track`] — the store behind history, frecency and the recorded outcome of a command.
//!
//! # What is not here yet
//!
//! **`Environment`.** It reads as though it belongs — a variable store depending on nothing — and
//! the crate plan says so. It does not: the store *holds* the shell's builtin table as a field and
//! `Environment::new()` fills it in, so it sits above the builtins rather than below them. Moving
//! it means deciding who constructs a shell environment, and that is a question about the shell,
//! not about this crate. It is left where it is until the answer is worth the churn.

pub mod ast;
/// The output of a command that was asked to keep it, for `copy --last`.
pub mod capture;
pub mod error;
/// Parts of the shell a config can turn off and on again while it runs.
pub mod feature;
pub mod hooks;
pub mod macros;
/// What this session said, kept after it has scrolled off.
pub mod messages;
/// The depth guard every parse passes through, and the heredoc scan that feeds alias expansion.
pub mod nesting;
#[cfg(feature = "vista")]
pub mod predict;
/// Whether the command now running must leave no trace of itself.
pub mod quiet;
/// A database a config or a plugin owns, kept apart from oslo's own.
/// Values kept encrypted, decrypted only when something asks for one.
#[cfg(feature = "secrets")]
pub mod secrets;
pub mod store;
pub mod track;
/// The shell's version, as one number rather than one per crate.
pub mod version;

pub use error::{Result, ShellError};
