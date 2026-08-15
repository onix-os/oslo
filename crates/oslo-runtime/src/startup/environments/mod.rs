//! Running the directory environment, which is the half [`oslo_shell::direnv`] cannot do itself.
//!
//! That module owns the lifecycle — what applies, whether it is allowed, and what to undo on the
//! way out — but running the file needs the Lua engine, which belongs to the read loop and not to a
//! module about directories. So this is where a `.env.lua` is actually run, and where the result is
//! reported.

mod live;
mod report;

use crate::lua::LuaEngine;
use oslo_shell::Environment;
use oslo_shell::direnv::find::{self, Rc};
use oslo_shell::direnv::{self, Direnv, Event};
use std::cell::RefCell;
use std::io::{Read, Seek, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::path::Path;
use std::sync::Mutex;

thread_local! {
    /// The prompt to put back when the loaded directory environment unloads.
    ///
    /// Thread-local because a Lua value is not `Send` and only the read loop's thread ever touches
    /// a prompt. `Some(None)` is meaningful and different from `None`: it says there was no prompt
    /// function before, so unloading must *remove* the directory's rather than leave it behind.
    static PREVIOUS_PROMPT: RefCell<Option<Option<crate::lua::eval::Value>>> =
        const { RefCell::new(None) };
}

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
pub(super) fn arrive(env: &Mutex<Environment>, lua: &LuaEngine, dir: &Path) {
    // **Every feature with an `oslo.feature.when` predicate is re-decided first**, for this
    // directory, before anything reads a feature bit.
    //
    // Here rather than at the three call sites, so that a fourth cannot forget it — and *before*
    // the load below rather than after, because the first thing a predicate is for is deciding
    // whether this directory's `.env.lua` should be loaded at all. That is also why this covers
    // the startup arrival: the directory a shell opens in is one a predicate has an opinion about
    // just as much as any other, and it is the one arrival no hook has fired for yet.
    crate::lua::api::feature::decide(dir);

    let mut said = String::new();
    // The prompt as it was before the directory that is loading got to touch it. Recorded at load
    // time and put back at unload time, which is the same record-and-restore shape the variables
    // use — it lives here rather than in `direnv` because a Lua value is not something a module
    // about directories should be holding.
    let mut base_prompt: Option<Option<crate::lua::eval::Value>> = None;
    let events = direnv::with(|state| {
        state.arrive(
            env,
            dir,
            &mut |rc| {
                base_prompt = Some(lua.prompt_handler());
                let (outcome, output) = capturing(|| match rc.kind {
                    find::Kind::Lua => source_lua(lua, rc),
                    find::Kind::Shell => source_envrc(env, rc),
                });
                said.push_str(&output);
                outcome
            },
            &mut || {
                if let Some(previous) = PREVIOUS_PROMPT.with(|slot| slot.borrow_mut().take()) {
                    lua.restore_prompt(previous);
                }
            },
        )
    });
    if let Some(base) = base_prompt {
        PREVIOUS_PROMPT.with(|slot| *slot.borrow_mut() = Some(base));
    }
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

/// Run `f` with stdout and stderr redirected, and hand back what it wrote *and has not shown*.
///
/// A temporary file rather than a pipe, deliberately: a pipe has a fixed capacity and nothing is
/// draining it while the rc file runs, so an `.envrc` chatty enough to fill it would block forever
/// on its own output. A file has no such limit and the whole thing is read once at the end.
///
/// **Read once at the end is not enough for a rc file that takes a minute.** `use flake` against a
/// cold store fetches and builds, saying so the whole way, and every word of that went into a file
/// nobody was reading — so the shell printed nothing until it was over and a long build was
/// indistinguishable from a hang. [`live`] tails the same file while `f` runs and puts what has
/// arrived on the real terminal; what it showed is subtracted here, so nothing is printed twice.
/// Nothing is shown at all for a file that finishes quickly.
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
    // Both measured before the redirect: afterwards stdout is the scratch file, which is not a
    // terminal and has no width. A shell whose output is piped shows no progress — there is nobody
    // watching it arrive, and it would land in the middle of somebody's data.
    let watching = std::io::IsTerminal::is_terminal(&std::io::stdout());
    let columns = oslo_ui::dropdown::width::terminal_cols();
    let _ = nix::unistd::dup2(scratch.as_raw_fd(), out);
    let _ = nix::unistd::dup2(scratch.as_raw_fd(), err);
    let live = watching.then(|| live::Live::start(&scratch, saved_out, columns));

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // Before the saved descriptors are reclaimed below, because the tail writes to one of them.
    let shown = live.map_or(0, live::Live::stop);
    // Reclaimed as owned so the duplicates are closed rather than leaked: this runs on every `cd`,
    // and a descriptor per directory change would exhaust the table in an afternoon.
    let saved_out = unsafe { std::os::fd::OwnedFd::from_raw_fd(saved_out) };
    let saved_err = unsafe { std::os::fd::OwnedFd::from_raw_fd(saved_err) };
    let _ = nix::unistd::dup2(saved_out.as_raw_fd(), out);
    let _ = nix::unistd::dup2(saved_err.as_raw_fd(), err);

    let mut said = String::new();
    let _ = scratch.seek(std::io::SeekFrom::Start(0));
    let _ = scratch.read_to_string(&mut said);
    // Only what the tail did not already put on screen. The boundary is a `\n` it counted, so it
    // lands on a character boundary; the check is there because a lossy decode of invalid UTF-8
    // could have moved it, and printing the lot twice beats slicing a string in half.
    let said = match said.is_char_boundary(shown) {
        true => said.split_off(shown),
        false => said,
    };

    match outcome {
        Ok(value) => (value, said),
        Err(panic) => {
            // Restored first, so the panic message is visible rather than written into the scratch
            // file nobody will read.
            std::panic::resume_unwind(panic)
        }
    }
}

/// Run an `.env.lua`, which may set more than variables.
fn source_lua(lua: &LuaEngine, rc: &Rc) -> Result<(), String> {
    let source = std::fs::read_to_string(&rc.path).map_err(|e| e.to_string())?;
    // **`oslo.mark()` inside one of these means the file's directory, not the shell's.** A
    // `.env.lua` governs everything below it, so walking straight into `app/src/deep` runs
    // `app/.env.lua` with the shell three levels down — and a mark that took the shell's word for
    // it would name `deep` after the project. Dropped on the way out, raise or no raise.
    let _loading = rc
        .path
        .parent()
        .map(crate::lua::api::mark::Loading::directory);
    lua.eval_as(&source, &rc.path.to_string_lossy())
        .map_err(|e| e.to_string())
}

/// Run an `.envrc`, which is shell, with direnv's stdlib in scope.
///
/// Three things have to be true for a file written for direnv to behave the way it does there, and
/// each one is a bug if it is missing:
///
/// * **the working directory is the file's own.** direnv `cd`s before it runs, and half the files
///   in the world say `PATH_add ./bin` or `dotenv .env` — resolved against wherever you happened to
///   be standing, those name nothing. Restored afterwards whatever happens, including on a panic,
///   because a shell left in a directory the user did not walk into is a far worse outcome than an
///   `.envrc` that failed.
/// * **the personal `direnvrc` is sourced first**, so the `use_` and `layout_` functions it defines
///   are there when the project's file calls them.
/// * **the stdlib is in scope only for this**, installed here and removed on the way out.
fn source_envrc(env: &Mutex<Environment>, rc: &Rc) -> Result<(), String> {
    let Some(home) = rc.path.parent().map(Path::to_path_buf) else {
        return Err("the file has no directory".to_string());
    };
    let came_from = std::env::current_dir().ok();
    std::env::set_current_dir(&home).map_err(|e| e.to_string())?;

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut guard = env.lock().map_err(|_| "the environment is locked")?;
        direnv::stdlib::install(&mut guard);
        direnv::rcfile::load(&mut guard);
        let status = oslo_shell::env::builtins::builtin_source(
            &mut guard,
            &["source".to_string(), rc.path.to_string_lossy().into_owned()],
        );
        direnv::stdlib::remove(&mut guard);
        match status {
            Ok(0) => Ok(()),
            Ok(code) => Err(format!("exited {code}")),
            Err(problem) => Err(problem.to_string()),
        }
    }));

    if let Some(back) = came_from {
        let _ = std::env::set_current_dir(back);
    }
    match outcome {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}
