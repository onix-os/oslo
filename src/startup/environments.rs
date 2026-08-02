//! Running the directory environment, which is the half [`oslo::direnv`] cannot do itself.
//!
//! That module owns the lifecycle — what applies, whether it is allowed, and what to undo on the
//! way out — but two of the three file types need an evaluator it has no business holding. `.envrc`
//! is shell and needs the executor; `.env.lua` is Lua and needs the engine. Both live up here, so
//! this is where they are run and where the result is reported.

use oslo::Environment;
use oslo::LuaEngine;
use oslo::direnv::find::{Kind, Rc};
use oslo::direnv::{self, Direnv, Event};
use oslo::exec::eval_command_list;
use oslo::parser::parse_with_aliases;
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
pub(super) fn arrive(env: &mut Environment, lua: &LuaEngine, dir: &Path) {
    let events = direnv::with(|state| {
        state.arrive(env, dir, &mut |env, rc| match rc.kind {
            Kind::Shell => source_shell(env, rc),
            Kind::Lua => source_lua(lua, rc),
            // Read by the module itself; never handed here.
            Kind::Dotenv => Ok(()),
        })
    });
    for event in events.unwrap_or_default() {
        report(&event);
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

/// Say what happened, in one line, on stderr.
///
/// stderr rather than stdout because none of it is a command's output, and a `cd` inside `$(...)`
/// must not have a "direnv: loaded" line captured into the substitution.
fn report(event: &Event) {
    match event {
        Event::Idle => {}
        Event::Loaded { owner, vars } => {
            eprintln!("direnv: loaded {} ({vars} variable(s))", owner.display());
        }
        Event::Unloaded { owner } => eprintln!("direnv: unloaded {}", owner.display()),
        Event::Blocked { path } => eprintln!(
            "direnv: {} is blocked — run `direnv allow` to trust it",
            path.display()
        ),
        Event::Denied { path } => eprintln!("direnv: {} is denied", path.display()),
        Event::Failed { path, problem } => {
            eprintln!("direnv: {}: {problem}", path.display());
        }
    }
}
