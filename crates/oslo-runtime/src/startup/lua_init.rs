//! The optional Lua configuration layer (PLAN R9.9).
//!
//! Two `let _ =` discards used to make a broken `init.lua` indistinguishable from no `init.lua`:
//! a typo on line 1 disabled every alias, prompt and binding in the file, and the shell started
//! as if the user had never written it. Config that fails must say so; it must not take the
//! shell down with it either, which is why nothing here is fatal.

use crate::lua::LuaEngine;
use oslo_base::error::ShellError;
use oslo_shell::Environment;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Where an interactive shell looks for its config, in order. The first that exists is used.
///
/// Three names for one file rather than three files that all load: a config split across places
/// is a config nobody can find the whole of, and "which one won" is the question that follows.
///
/// Configuration is Lua and nothing else. `~/.oslorc` used to be looked for here too; anyone
/// with an old one gets a Lua syntax error, which is loud and points at the line — the loudest
/// available way to say the format changed, and better than half of it silently working.
pub fn config_paths(env: &Environment) -> Vec<PathBuf> {
    let home = env
        .get_var("HOME")
        .map(str::to_string)
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default();

    let xdg = env
        .get_var("XDG_CONFIG_HOME")
        .map(str::to_string)
        .or_else(|| std::env::var("XDG_CONFIG_HOME").ok())
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .or_else(|| (!home.is_empty()).then(|| PathBuf::from(&home).join(".config")));

    // **One name, one language.** `oslo/config` without the extension and `~/.oslorc` both used to
    // be looked for as well. Neither is worth the question they create — "which one is being read,
    // and what happens if I have two?" — and `.oslorc` in particular reads like a shell rc file to
    // anyone who has used one, which it has not been for some time. Configuration is Lua, the file
    // says so in its name, and there is exactly one place to put it.
    //
    // `init.lua`, because the directory is a Lua *package* and that is what a package's entry point
    // is called — the same name nvim uses, and the one `require "oslo"` and `require "aliases"`
    // already imply. `?/init.lua` is on `package.path` for exactly this reason.
    let mut paths = Vec::new();
    if let Some(xdg) = xdg {
        paths.push(xdg.join("oslo/init.lua"));
    }
    let _ = home;
    paths
}

/// The config file this shell will actually read, if there is one.
pub fn config_path(env: &Environment) -> Option<PathBuf> {
    config_paths(env).into_iter().find(|p| p.is_file())
}

/// Every `.lua` file in `oslo/conf.d`, in name order, then the config file itself.
///
/// fish's `conf.d`, and it exists for the same reason fish grew it: a plugin, a package manager or
/// a dotfile repo needs somewhere to add a line of configuration **without editing a file it does
/// not own**. Appending to `init.lua` means every uninstall is a text edit that can go wrong, and
/// two tools appending at once means a merge conflict in a file a person also writes by hand.
///
/// Name order rather than directory order, so `10-path.lua` runs before `20-prompt.lua` and the
/// result does not depend on what the filesystem feels like returning. `init.lua` runs **last**,
/// so the file you wrote by hand has the final say over anything a package dropped in.
pub fn config_files(env: &Environment) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Some(dir) = config_paths(env).first().and_then(|p| p.parent()) {
        let mut snippets: Vec<PathBuf> = std::fs::read_dir(dir.join("conf.d"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|e| e == "lua") && path.is_file())
            .collect();
        snippets.sort();
        files.extend(snippets);
    }
    files.extend(config_path(env));
    files
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

/// Run the config if there is one, reporting a broken one as `oslo: <path>: <error>`.
///
/// The shell carries on with its defaults afterwards either way: a config that fails half way
/// leaves whatever it managed to set, and a broken one must not cost you your shell.
pub fn load_config(lua: &LuaEngine, path: &Path) {
    if !path.is_file() {
        return;
    }
    // `load_file` still takes a `&str`; a path that is not UTF-8 is reported rather than
    // `unwrap`ped, which is what used to panic the shell before it printed its first prompt.
    let Some(text) = path.to_str() else {
        oslo_base::messages::error(path.display().to_string(), "path is not valid UTF-8");
        return;
    };
    if let Err(e) = lua.load_file(text) {
        // Printed with the path marked up and remembered without the escape codes: a config that
        // failed at startup is the thing most often still being asked about twenty commands later.
        eprintln!(
            "oslo: {}: {}",
            oslo_ui::marks::path(&path.display().to_string()),
            e
        );
        oslo_base::messages::say(
            oslo_base::messages::Level::Error,
            path.display().to_string(),
            e.to_string(),
        );
    }
}

/// The status `oslo.proc.exit(n)` asked for, if that is what ended the script.
///
/// The request travels as an error because unwinding is the only way out of a call several Lua
/// frames deep, and the status rides on the error itself rather than being recovered from a
/// message. That is what makes `oslo.proc.exit` work from inside a function, a callback, or a
/// registered builtin, rather than only at the top level of a script.
fn requested_exit(err: &ShellError) -> Option<i32> {
    match err {
        ShellError::Lua(lua_err) => lua_err.exit,
        _ => None,
    }
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
/// "Its status" is `$?` as the script leaves it, so `oslo.proc.exec("false")` at the end of the file
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
        // `oslo.proc.exit(n)` unwinds as a shell exit rather than a Lua failure. Without this it
        // reached here as an ordinary error and printed a traceback, so the one API for choosing
        // an exit status produced a diagnostic and exit 1 instead.
        if let Some(code) = requested_exit(&e) {
            return code;
        }
        // The error names its own file, and with a *line* where the VM knew one — `e.lua:3: …`
        // rather than the `e.lua: ` this used to put in front of it. Naming the script here as
        // well printed it twice for every failure.
        eprintln!("oslo: {e}");
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
