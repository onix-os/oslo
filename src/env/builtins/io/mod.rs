//! Reading and writing: `echo` and `read`.
//!
//! `read` is three separable problems and is split along those seams rather than by size:
//! [`read_input`] owns the bytes (which descriptor, which delimiter, how many characters, how
//! long to wait), [`read_split`] owns the `IFS` field semantics that turn one line into named
//! variables, and [`read`] owns the option grammar that connects them.

mod echo;
mod printf;
#[allow(clippy::module_inception)]
mod read;
mod read_input;
mod read_split;

pub use echo::builtin_echo;
pub use printf::builtin_printf;
pub use read::builtin_read;
