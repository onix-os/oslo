//! What the prompt says, in whichever language the line is being read as.
//!
//! Split from `repl` because it is a separate question from *reading* a line: the loop asks for a
//! string and draws it, and everything about where that string comes from — a Lua function, `PS1`,
//! or the built-in default — is decided here.

use super::mode::Mode;
use super::rc;
use crate::lua::LuaEngine;
use oslo_shell::Environment;
use std::sync::{Arc, Mutex};

/// The facts a prompt segment is drawn from, gathered once.
///
/// Everything here the shell already knows; a segment looking each of them up itself would run
/// `git` once per segment per keystroke.
pub fn segment_context(
    last_status: i32,
    mode: Mode,
    vimode: Option<&str>,
) -> crate::lua::context::Context {
    crate::lua::context::Context {
        status: last_status,
        duration_ms: super::repl::last_command_duration().map(|d| d.as_millis() as u64),
        cwd: super::repl::cwd(),
        branch: oslo_ui::prompt::git_branch(),
        user: whoami(),
        host: hostname(),
        language: mode.name().to_string(),
        vimode: vimode
            .map(str::to_string)
            .or_else(|| oslo_ui::vi::mode().map(|m| m.name().to_string())),
        cols: oslo_ui::dropdown::terminal_cols(),
        // The real count. Hardcoded `0` until now, which made a `jobs` segment in a prompt — the
        // reason the field exists — always draw nothing. Every prompt tool reads this; in bash it
        // is `jobs -p | wc -l` and in zsh `${#jobstates}`.
        jobs: oslo_shell::exec::job::with_jobs(|jobs| jobs.jobs().len()),
        continuation: false,
        // Nothing is running at a prompt; `title_context` fills this in for the other case.
        command: None,
    }
}

/// The same facts, for a title drawn while `command` is running.
pub fn title_context(last_status: i32, mode: Mode, command: &str) -> crate::lua::context::Context {
    crate::lua::context::Context {
        command: Some(command.to_string()),
        ..segment_context(last_status, mode, None)
    }
}

/// Who is logged in, `$USER` first because that is what `su` updates.
fn whoami() -> String {
    std::env::var("USER")
        .ok()
        .filter(|u| !u.is_empty())
        .or_else(|| {
            nix::unistd::User::from_uid(nix::unistd::getuid())
                .ok()
                .flatten()
                .map(|u| u.name)
        })
        .unwrap_or_else(|| "?".to_string())
}

/// This machine's short name — everything before the first dot.
fn hostname() -> String {
    nix::unistd::gethostname()
        .ok()
        .and_then(|h| h.into_string().ok())
        .map(|h| h.split('.').next().unwrap_or(&h).to_string())
        .unwrap_or_else(|| "?".to_string())
}

pub fn primary_prompt(
    env_struct: &Arc<Mutex<Environment>>,
    lua: &LuaEngine,
    last_status: i32,
    mode: Mode,
) -> String {
    // Published before the prompt is drawn, so a `PS1` or a Lua prompt function can say which
    // language it is prompting for.
    env_struct
        .lock()
        .unwrap()
        .set_var("OSLO_MODE", mode.name(), false);

    // A Lua prompt is an explicit choice by the user and outranks `PS1`, which in turn outranks
    // the built-in default.
    lua.render_with("prompt.left", &segment_context(last_status, mode, None))
        .or_else(|| lua.render_prompt())
        .unwrap_or_else(|| {
            // Both languages get the *same* prompt, with the language as one of its segments —
            // `you@host | N | lua >`. A separate `lua>` used to be the only signal, which meant
            // switching language threw away the branch, the mode and the directory as well.
            //
            // `PS1` still wins for shell lines, because that is what `PS1` is. It cannot win for
            // Lua ones: it describes a shell prompt, and drawing `oslo$` in front of something
            // that is not a shell command is exactly the confusion this segment exists to stop.
            if mode == Mode::Lua {
                oslo_ui::prompt::render_default_left_prompt(last_status, mode.name())
            } else {
                rc::ps1(&mut env_struct.lock().unwrap(), last_status)
            }
        })
}
