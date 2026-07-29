//! The optional Lua configuration layer (PLAN R9.9).
//!
//! Two `let _ =` discards used to make a broken `init.lua` indistinguishable from no `init.lua`:
//! a typo on line 1 disabled every alias, prompt and binding in the file, and the shell started
//! as if the user had never written it. Config that fails must say so; it must not take the
//! shell down with it either, which is why nothing here is fatal.

use rush::Environment;
use rush::LuaEngine;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where an interactive shell looks for its Lua layer.
pub fn init_lua_path(env: &Environment) -> Option<PathBuf> {
    let home = env
        .get_var("HOME")
        .map(str::to_string)
        .or_else(|| std::env::var("HOME").ok())?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config/rush/init.lua"))
}

/// Wire the `rush.*` table into `lua`, reporting a failure instead of swallowing it.
///
/// Returns whether the bindings are usable. When they are not, `init.lua` is not run at all:
/// every line in it would fail on `rush` being nil, and one clear message beats fifty.
pub fn install_bindings(lua: &LuaEngine, env: Arc<Mutex<Environment>>) -> bool {
    match lua.setup_bindings(env) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("rush: lua: cannot install the rush bindings: {}", e);
            false
        }
    }
}

/// Run `init.lua` if there is one, reporting a broken one as `rush: <path>: <error>`.
///
/// The shell carries on with its defaults afterwards either way.
pub fn load_init_lua(lua: &LuaEngine, path: &Path) {
    if !path.is_file() {
        return;
    }
    // `load_file` still takes a `&str`; a path that is not UTF-8 is reported rather than
    // `unwrap`ped, which is what used to panic the shell before it printed its first prompt.
    let Some(text) = path.to_str() else {
        eprintln!("rush: {}: path is not valid UTF-8", path.display());
        return;
    };
    if let Err(e) = lua.load_file(text) {
        eprintln!("rush: {}: {}", path.display(), e);
    }
}

/// `--lua-script FILE`: run a Lua script instead of a shell one, and exit with its status.
///
/// "Its status" is `$?` as the script leaves it, so `rush.exec("false")` at the end of the file
/// exits 1. Before this, the process exited 0 no matter what the script ran, which made
/// `--lua-script` unusable from anything that checks an exit code.
pub fn run_lua_script(path: &str) -> i32 {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = match LuaEngine::new() {
        Ok(lua) => lua,
        Err(e) => {
            eprintln!("rush: lua: {}", e);
            return 1;
        }
    };
    if !install_bindings(&lua, Arc::clone(&env)) {
        return 1;
    }
    if let Err(e) = lua.load_file(path) {
        eprintln!("rush: {}: {}", path, e);
        return 1;
    }
    env.lock().map(|guard| guard.last_status).unwrap_or(1)
}
