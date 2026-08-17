//! The standard names oslo refuses, and what it says instead.
//!
//! **Three functions the language provides that this shell must not.** They are not missing and not
//! broken — the VM implements all three correctly — they are *refused*, because of what they do:
//!
//! * `os.execute` and `io.popen` run their argument through `/bin/sh`. That is someone else's shell,
//!   started from inside this one, with its own quoting rules and its own idea of what a word is —
//!   and on a machine where oslo is the only shell installed it is nothing at all. `oslo.run` takes
//!   an argv and needs no shell to interpret it.
//! * `os.tmpname` returns a *name* and creates nothing, so between the name and the open there is a
//!   window for somebody else to get there first. `oslo.fs.mktemp` creates the file and hands back
//!   the path, which is the same call without the race.
//!
//! Each refusal names its replacement, because a refusal a reader cannot act on is just a wall.
//! `os.execute()` with no argument still answers `true`: it asks whether a shell is available
//! rather than running anything, and one is.

use super::util::native;
use oslo_base::value::{LuaError, Value};
use oslo_luavm::Host;

/// Where `require` looks, and where it must not.
///
/// **Stock Lua's default ends in `./?.lua`.** In a shell that is a script hijack: `cd` into a
/// directory somebody else can write, run anything that requires a module, and load their
/// `utils.lua` instead of yours. oslo's path is its own config directory and the system ones, and
/// the working directory is not on it.
///
/// **The config directory itself is on the path, not only `lua/` under it.** A configuration that
/// has grown a second file — a prompt, a set of key bindings — should join it with
/// `require "aliases"`, and that only works if the directory the file is *in* is searched. Without
/// it the only way to reach a sibling was
/// `dofile(oslo.env.get("HOME") .. "/.config/oslo/aliases.lua")`: an absolute path, spelled out, in
/// a language that has had a module system for twenty years. `lua/` is still searched after it, for
/// a library kept apart from the configuration that uses it.
///
/// `cpath` is set to the empty string rather than left unset. A static binary cannot `dlopen`
/// anything, so advertising a C path would turn an honest "module not found" into a confusing
/// loader error — but a `cpath` that is *absent* breaks `package.cpath == ""`, which is the way a
/// script asks whether C modules are available at all.
fn search_path() -> String {
    let config = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|x| !x.is_empty())
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.config")));
    let mut parts = Vec::new();
    if let Some(config) = config {
        parts.push(format!("{config}/oslo/?.lua"));
        parts.push(format!("{config}/oslo/?/init.lua"));
        parts.push(format!("{config}/oslo/lua/?.lua"));
        parts.push(format!("{config}/oslo/lua/?/init.lua"));
    }
    parts.push("/usr/local/share/lua/5.4/?.lua".to_string());
    parts.push("/usr/local/share/lua/5.4/?/init.lua".to_string());
    parts.push("/usr/share/lua/5.4/?.lua".to_string());
    parts.push("/usr/share/lua/5.4/?/init.lua".to_string());
    parts.join(";")
}

/// Replace the three names, in the tables the VM actually holds.
pub(super) fn apply(host: &dyn Host) {
    host.set_field(&["package", "path"], Value::str(search_path()));
    host.set_field(&["package", "cpath"], Value::str(""));

    host.set_field(
        &["os", "execute"],
        native("os.execute", |_, args| {
            // No argument is the "is there a shell?" question, not a request to run one.
            if args.first().is_none_or(|v| matches!(v, Value::Nil)) {
                return Ok(vec![Value::Bool(true)]);
            }
            Err(LuaError::new(REPLACE_WITH_RUN))
        }),
    );
    host.set_field(
        &["io", "popen"],
        native("io.popen", |_, _| Err(LuaError::new(REPLACE_WITH_RUN))),
    );
    host.set_field(
        &["os", "tmpname"],
        native("os.tmpname", |_, _| {
            Err(LuaError::new(
                "os.tmpname names a file without creating it, so the name can be taken before you \
                 open it — use oslo.fs.mktemp, which creates the file and answers its path",
            ))
        }),
    );
}

const REPLACE_WITH_RUN: &str = "this runs its argument through /bin/sh — another shell, from inside this one, and absent \
     entirely where oslo is the only one installed. Use oslo.run{\"cmd\", \"arg\"}, which takes an \
     argv and needs no shell to read it.";
