//! `make` — a builtin, because a `.make.lua` is a thing only this shell can see.
//!
//! # It gets out of the way, and that is most of what it does
//!
//! `make` is a real program that a great many projects in the world are built with, and shadowing it
//! would be the mistake this codebase already refuses for `test`: a shell that promises scripts see
//! POSIX behaviour cannot have something on disk quietly redefining a command. The precedent to
//! copy is [`super::locate`], whose `which` answers about *this* shell at a prompt and hands the
//! word to `/usr/bin/which` everywhere else.
//!
//! So three conditions, and every one of them must hold before this builtin means anything:
//!
//! 1. **The shell is interactive.** In a script, `make` is the program the script was written for.
//! 2. **A `.make.lua` governs the working directory.** No file, no claim on the name.
//! 3. `\make` and `command make` still reach the program, as they do for every builtin.
//!
//! A project with a `Makefile` *and* a `.make.lua` gets the Lua one at a prompt — the same rule
//! `direnv::find` applies for `.env.lua`, for the same stated reason: a
//! repository holding both almost always has one of them for everybody else.
//!
//! # Why it runs a child rather than doing the work
//!
//! A recipe has to be able to run commands, and Lua called from inside a builtin cannot: the shell
//! is holding its own state, so `oslo.run` fails with *shell state is busy*. `oslo make` is a
//! separate process for exactly that reason — see `src/cli/make.rs`. This builtin's whole job is to
//! decide whether the word is oslo's, and then to become that process.

use crate::env::origin_now;
use crate::env::scope::Environment;
use oslo_base::error::Result;

/// `make [OPTIONS] [RECIPE [ARGS...]]`
pub fn builtin_make(env: &mut Environment, args: &[String]) -> Result<i32> {
    if !env.interactive() || governing_file().is_none() {
        return handover(args);
    }
    // The tool, in a child of this shell: `oslo make …`. `current_exe` rather than a `$PATH` search
    // for `oslo`, because the shell you are typing at is the one whose recipes you mean — a
    // different build earlier on `$PATH` would answer with a different runner.
    let Ok(exe) = std::env::current_exe() else {
        return handover(args);
    };
    let mut argv = vec!["oslo".to_string(), "make".to_string()];
    argv.extend(args.iter().skip(1).cloned());
    super::spawn::run_external(&exe, &argv, "make")
}

/// The `.make.lua` for the working directory, if there is one.
fn governing_file() -> Option<std::path::PathBuf> {
    let here = std::env::current_dir().ok()?;
    crate::make::governing(&here)
}

/// Run the program by this name instead.
///
/// **And say so when there is none.** Without a `Makefile` in sight and without the program
/// installed, "make: command not found" is the honest answer and the one every other shell gives —
/// but a person who has just written a `.make.lua` one directory up and walked out of it deserves
/// better than silence, which is what the hint is for.
fn handover(args: &[String]) -> Result<i32> {
    match super::spawn::resolve_program("make") {
        Some(program) => super::spawn::run_external(&program, args, "make"),
        None => {
            eprintln!("{}make: command not found", origin_now());
            eprintln!(
                "make: no {} governs this directory, and no `make` program is installed",
                crate::make::NAME
            );
            Ok(127)
        }
    }
}

#[cfg(test)]
#[path = "make/tests.rs"]
mod tests;
