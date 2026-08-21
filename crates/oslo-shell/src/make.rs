//! Which directory's recipes govern this one.
//!
//! The whole of the shell's half. Everything that *runs* a recipe lives in `oslo-runtime`, because
//! running one means a Lua engine, and a module about finding a file has no business holding one.
//!
//! # Why the rule is direnv's, verbatim
//!
//! Walk **up** and take the nearest ancestor that has a `.make.lua`. Nearest, not all: two files on
//! one path means the inner one governs and the outer one does not, so what `make build` does never
//! depends on how deep in the tree you happened to be standing. That is the same argument
//! [`crate::direnv::find`] makes for `.env.lua`, and repeating it here rather than sharing the code
//! is deliberate — the two files answer different questions and only coincidentally agree on how to
//! be found.
//!
//! # It is not read here, and not on arrival
//!
//! `.env.lua` is read when you walk into a directory, which is why it needs an allow gate. A
//! `.make.lua` is read when you type `make`, which is what `make` has always been. So this module
//! answers *where the file is* and never opens it: the decision to run it is the caller's, and the
//! caller is a person who asked.

use std::path::{Path, PathBuf};

/// The file a project keeps its recipes in.
pub const NAME: &str = ".make.lua";

/// The `.make.lua` governing `dir`, from the nearest ancestor that has one.
///
/// `None` for almost every directory on the machine, so the cost that matters is the miss: one
/// `stat` per ancestor and nothing else. No canonicalisation — a symlinked checkout should answer
/// for the path you are standing in, not for the one it points at.
pub fn governing(dir: &Path) -> Option<PathBuf> {
    for ancestor in dir.ancestors() {
        let candidate = ancestor.join(NAME);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The directory a recipe runs in: the one holding the file.
///
/// make's rule and just's both. A recipe that resolved `src/` against wherever you happened to be
/// standing would mean something different from two directories down, which is the one thing a
/// checked-in build file cannot afford.
pub fn root_of(file: &Path) -> Option<&Path> {
    file.parent()
}

#[cfg(test)]
#[path = "make/tests.rs"]
mod tests;
