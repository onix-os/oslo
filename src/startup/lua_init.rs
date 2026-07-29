//! The optional Lua configuration layer (PLAN R9.9).
//!
//! Two `let _ =` discards used to make a broken `init.lua` indistinguishable from no `init.lua`:
//! a typo on line 1 disabled every alias, prompt and binding in the file, and the shell started
//! as if the user had never written it. Config that fails must say so; it must not take the
//! shell down with it either, which is why nothing here is fatal.

use oslo::Environment;
use oslo::LuaEngine;
use oslo::error::ShellError;
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
    Some(PathBuf::from(home).join(".config/oslo/init.lua"))
}

/// Wire the `oslo.*` table into `lua`, reporting a failure instead of swallowing it.
///
/// Returns whether the bindings are usable. When they are not, `init.lua` is not run at all:
/// every line in it would fail on `oslo` being nil, and one clear message beats fifty.
pub fn install_bindings(lua: &LuaEngine, env: Arc<Mutex<Environment>>) -> bool {
    match lua.setup_bindings(env) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("oslo: lua: cannot install the oslo bindings: {}", e);
            false
        }
    }
}

/// Run `init.lua` if there is one, reporting a broken one as `oslo: <path>: <error>`.
///
/// The shell carries on with its defaults afterwards either way.
pub fn load_init_lua(lua: &LuaEngine, path: &Path) {
    if !path.is_file() {
        return;
    }
    // `load_file` still takes a `&str`; a path that is not UTF-8 is reported rather than
    // `unwrap`ped, which is what used to panic the shell before it printed its first prompt.
    let Some(text) = path.to_str() else {
        eprintln!("oslo: {}: path is not valid UTF-8", path.display());
        return;
    };
    if let Err(e) = lua.load_file(text) {
        eprintln!("oslo: {}: {}", path.display(), e);
    }
}

/// The status `oslo.exit(n)` asked for, if that is what ended the script.
///
/// The request travels as a `ShellError::Exit` wrapped by mlua, and mlua nests: a callback error
/// carries its cause, which may itself be wrapped with context. Walking the chain is what makes
/// `oslo.exit` work from inside a function, a `pcall`-free callback, or a registered builtin,
/// rather than only at the top level of a script.
fn requested_exit(err: &ShellError) -> Option<i32> {
    let ShellError::Lua(lua_err) = err else {
        return None;
    };
    fn walk(e: &mlua::Error) -> Option<i32> {
        match e {
            mlua::Error::CallbackError { cause, .. } => walk(cause),
            mlua::Error::WithContext { cause, .. } => walk(cause),
            mlua::Error::ExternalError(inner) => match inner.downcast_ref::<ShellError>() {
                Some(ShellError::Exit(code)) => Some(*code),
                _ => None,
            },
            _ => None,
        }
    }
    walk(lua_err)
}

/// Blank out a leading `#!` line so Lua can parse the source.
///
/// Lua's *file* loader skips a shebang; loading the same bytes as a string does not, so
/// `#!/usr/bin/env lua` reaches the parser and dies with "unexpected symbol near '#'". The line is
/// replaced rather than removed so every later line keeps its number and a traceback still points
/// at the right place.
fn without_shebang(source: &str) -> String {
    match source.strip_prefix("#!") {
        None => source.to_string(),
        Some(rest) => match rest.find('\n') {
            Some(end) => format!("\n{}", &rest[end + 1..]),
            None => String::new(),
        },
    }
}

/// Run Lua source as the shell's program, and exit with its status.
///
/// "Its status" is `$?` as the script leaves it, so `oslo.exec("false")` at the end of the file
/// exits 1; the process used to exit 0 no matter what the script ran, which made this unusable
/// from anything that checks an exit code.
///
/// `name` is what a diagnostic calls the source — a path, or `-c`/`stdin` when there is no file.
pub fn run_lua_source(source: &str, name: &str, args: &[String]) -> i32 {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = match LuaEngine::new() {
        Ok(lua) => lua,
        Err(e) => {
            eprintln!("oslo: lua: {}", e);
            return 1;
        }
    };
    if !install_bindings(&lua, Arc::clone(&env)) {
        return 1;
    }
    if let Err(e) = lua.set_script_args(name, args) {
        eprintln!("oslo: lua: {}", e);
        return 1;
    }
    if let Err(e) = lua.eval_as(&without_shebang(source), name) {
        // `oslo.exit(n)` unwinds as a shell exit rather than a Lua failure. Without this it
        // reached here as an ordinary error and printed a traceback, so the one API for choosing
        // an exit status produced a diagnostic and exit 1 instead.
        if let Some(code) = requested_exit(&e) {
            return code;
        }
        eprintln!("oslo: {}: {}", name, e);
        return 1;
    }
    env.lock().map(|guard| guard.last_status).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::without_shebang;

    /// A shebanged Lua script has to parse, and its line numbers have to survive: Lua's file
    /// loader skips `#!` but its string loader does not, and switching to the latter is what
    /// made `oslo script` able to detect the language at all.
    #[test]
    fn a_shebang_is_blanked_out_without_shifting_the_lines() {
        let out = without_shebang("#!/usr/bin/env lua\nprint(1)\n");
        assert_eq!(out, "\nprint(1)\n");
        assert_eq!(out.lines().count(), 2, "line numbers must not shift");
    }

    #[test]
    fn source_without_a_shebang_is_untouched() {
        assert_eq!(without_shebang("print(1)\n"), "print(1)\n");
        // A `#` that is not a shebang is Lua's length operator and must survive.
        assert_eq!(without_shebang("print(#t)\n"), "print(#t)\n");
    }

    #[test]
    fn a_shebang_with_no_newline_leaves_nothing_to_run() {
        assert_eq!(without_shebang("#!/usr/bin/lua"), "");
    }
}
