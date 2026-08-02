//! Running the directory environment, which is the half [`oslo::direnv`] cannot do itself.
//!
//! That module owns the lifecycle — what applies, whether it is allowed, and what to undo on the
//! way out — but two of the three file types need an evaluator it has no business holding. `.envrc`
//! is shell and needs the executor; `.env.lua` is Lua and needs the engine. Both live up here, so
//! this is where they are run and where the result is reported.

mod report;

use oslo::Environment;
use oslo::LuaEngine;
use oslo::direnv::find::{Kind, Rc};
use oslo::direnv::{self, Direnv, Event};
use oslo::exec::eval_command_list;
use oslo::parser::parse_with_aliases;
use std::io::{Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::Path;

/// Give an interactive shell its directory environment.
///
/// Interactive only, and that is the same rule the tracking store follows: a script's environment
/// comes from whoever ran it, and a file sitting in the working directory quietly changing it would
/// make scripts behave differently depending on where they were invoked from.
pub(super) fn start() {
    direnv::install(Direnv::new(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    ));
}

/// Bring the environment into line with `dir`, running whatever needs running, and report.
///
/// The rc file's own output is captured and printed *under* the line naming the file, rather than
/// being left to land above a summary that does not mention it. On a real `.envrc` that is the
/// difference between eight loose `command not found` lines and one labelled block.
pub(super) fn arrive(env: &mut Environment, lua: &LuaEngine, dir: &Path) {
    let mut said = String::new();
    let events = direnv::with(|state| {
        state.arrive(env, dir, &mut |env, rc| {
            let (outcome, output) = capturing(|| match rc.kind {
                Kind::Shell => source_shell(env, rc),
                Kind::Lua => source_lua(lua, rc),
                // Read by the module itself; never handed here.
                Kind::Dotenv => Ok(()),
            });
            said.push_str(&output);
            outcome
        })
    });
    for event in events.unwrap_or_default() {
        report::event(&event);
        // The detail belongs under the failure it explains, and nowhere else — a successful load
        // that printed nothing should not be followed by an empty block.
        if matches!(event, Event::Failed { .. }) && !said.trim().is_empty() {
            report::detail(&said);
            said.clear();
        }
    }
    // Output from a file that did not fail is still the file's, and still worth showing: an
    // `.envrc` that prints a warning and succeeds has said something the user chose to say.
    if !said.trim().is_empty() {
        report::detail(&said);
    }
}

/// Run `f` with stdout and stderr redirected, and hand back what it wrote.
///
/// A temporary file rather than a pipe, deliberately: a pipe has a fixed capacity and nothing is
/// draining it while the rc file runs, so an `.envrc` chatty enough to fill it would block forever
/// on its own output. A file has no such limit and the whole thing is read once at the end.
///
/// Both descriptors are restored whatever happens, including when the evaluator panics, because
/// leaving the shell's stdout pointing at a deleted temp file would silently swallow everything
/// typed afterwards.
fn capturing<T>(f: impl FnOnce() -> T) -> (T, String) {
    let _ = std::io::stdout().flush();
    let Ok(mut scratch) = tempfile::tempfile() else {
        return (f(), String::new());
    };
    let out = std::io::stdout().as_raw_fd();
    let err = std::io::stderr().as_raw_fd();
    let (Ok(saved_out), Ok(saved_err)) = (nix::unistd::dup(out), nix::unistd::dup(err)) else {
        return (f(), String::new());
    };
    let _ = nix::unistd::dup2(scratch.as_raw_fd(), out);
    let _ = nix::unistd::dup2(scratch.as_raw_fd(), err);

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // Reclaimed as owned so the duplicates are closed rather than leaked: this runs on every `cd`,
    // and a descriptor per directory change would exhaust the table in an afternoon.
    let saved_out = unsafe { std::os::fd::OwnedFd::from_raw_fd(saved_out) };
    let saved_err = unsafe { std::os::fd::OwnedFd::from_raw_fd(saved_err) };
    let _ = nix::unistd::dup2(saved_out.as_raw_fd(), out);
    let _ = nix::unistd::dup2(saved_err.as_raw_fd(), err);

    let mut said = String::new();
    let _ = scratch.seek(std::io::SeekFrom::Start(0));
    let _ = scratch.read_to_string(&mut said);

    match outcome {
        Ok(value) => (value, said),
        Err(panic) => {
            // Restored first, so the panic message is visible rather than written into the scratch
            // file nobody will read.
            std::panic::resume_unwind(panic)
        }
    }
}

/// Run an `.envrc` on oslo's own evaluator.
///
/// **Not a subshell, and not bash.** direnv runs `.envrc` under a bash it spawns, then diffs the
/// environment that comes back — it has no other option, being a separate process. Running it here
/// means an `.envrc` is written in the same shell the user is typing into, and that `export FOO=1`
/// simply *is* an export, with no round trip to serialise it through.
///
/// The cost is that an `.envrc` written for the real direnv will use its stdlib — `use flake`,
/// `layout python`, `export_alias` — and those are functions oslo does not have. They fail as
/// unknown commands, loudly, which is the honest outcome: the alternative is a half-supported
/// stdlib where `layout python` silently does nothing and the user hunts for why their virtualenv
/// is missing.
fn source_shell(env: &mut Environment, rc: &Rc) -> Result<(), String> {
    let source = std::fs::read_to_string(&rc.path).map_err(|e| e.to_string())?;
    let ast = parse_with_aliases(&source, &|name| env.get_alias(name).map(str::to_string))
        .map_err(|e| e.to_string())?;
    match eval_command_list(env, &ast) {
        Ok(0) => Ok(()),
        // A non-zero status from an rc file is worth saying: it is how a `use flake` that oslo
        // cannot run announces itself, and swallowing it is how a directory environment silently
        // half-applies.
        Ok(status) => Err(format!("exited with status {status}")),
        Err(problem) => Err(problem.to_string()),
    }
}

/// Run an `.env.lua`, which may set more than variables.
fn source_lua(lua: &LuaEngine, rc: &Rc) -> Result<(), String> {
    let source = std::fs::read_to_string(&rc.path).map_err(|e| e.to_string())?;
    lua.eval_as(&source, &rc.path.to_string_lossy())
        .map_err(|e| e.to_string())
}
