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
            // Both languages get the *same* prompt, with the language as one of its segments —
            // `you@host | N | lua ❯`. A separate `lua>` used to be the only signal, which meant
            // switching language threw away the branch, the mode and the directory as well.
            //
            // `PS1` still wins for shell lines, because that is what `PS1` is. It cannot win for
            // Lua ones: it describes a shell prompt, and drawing `oslo$` in front of something
            // that is not a shell command is exactly the confusion this segment exists to stop.
            if mode == Mode::Lua {
                oslo::interactive::prompt::render_default_left_prompt(last_status, mode.name())
            } else {
                rc::ps1(&mut env_struct.lock().unwrap(), last_status)
            }
        })
}
