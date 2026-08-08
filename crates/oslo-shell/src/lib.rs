//! The shell itself: what turns a line of text into something that ran.
//!
//! Six modules that cannot usefully be told apart, and one that could be but is not worth it:
//!
//! * [`syntax`] — brush's AST converted into oslo's, plus alias expansion.
//! * [`lexer`] — the word scanner the adapter re-lexes through, and the arithmetic and heredoc
//!   scanners that grew out of it.
//! * [`expand`] — parameters, globs, braces, arithmetic: a word becoming its fields.
//! * [`exec`] — running what came out, including jobs, pipelines and redirection.
//! * [`mod@env`] — the variable store, the options, and every builtin.
//! * [`data`] — the structured pipeline: tools that produce rows rather than bytes.
//! * [`direnv`] — directory environments, `.env.lua` and `.envrc`.
//!
//! # Why `exec` and `env` are one crate
//!
//! They call each other. Running a simple command dispatches to a builtin; builtins call back into
//! `exec` for subshells, `eval` and command substitution — fourteen references across ten files,
//! measured. That is mutual recursion in the problem rather than in the code, and no arrangement
//! of directories removes it. It is why this crate is the large one.
//!
//! # What it took to get here
//!
//! Three edges pointed up at the Lua API and had to be turned around first, all from `data`:
//! the system-fact producers moved down here where their dependencies already were; the registry
//! of config-supplied tools moved to [`data::custom`], which the pipeline reads and the API writes;
//! and "the interpreter parked on this thread" moved into `oslo-lua`, which owns the type.

pub mod data;
pub mod direnv;
pub mod env;
pub mod exec;
pub mod expand;
pub mod lexer;
/// The brush→oslo adapter and the nesting guard. There is one shell parser and it is brush's;
/// this is the conversion into oslo's own tree, which is why it is not called `parser`.
pub mod syntax;

pub use env::Environment;
pub use exec::{JobManager, eval_command_list};
pub use lexer::Lexer;
pub use syntax::parse_bash_script;
