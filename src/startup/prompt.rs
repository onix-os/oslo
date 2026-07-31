//! What the prompt says, in whichever language the line is being read as.
//!
//! Split from `repl` because it is a separate question from *reading* a line: the loop asks for a
//! string and draws it, and everything about where that string comes from — a Lua function, `PS1`,
//! or the built-in default — is decided here.

use super::mode::Mode;
use super::rc;
use oslo::Environment;
use oslo::LuaEngine;
use std::sync::{Arc, Mutex};

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
    lua.render("prompt.left")
        .or_else(|| lua.render_prompt())
        .unwrap_or_else(|| {
            // `PS1` is the shell's prompt and describes a shell line; drawing it over a Lua
            // prompt would say `oslo$` in front of something that is not a shell command.
            if mode == Mode::Lua {
                mode.fallback_prompt().to_string()
            } else {
                rc::ps1(&mut env_struct.lock().unwrap(), last_status)
            }
        })
}
