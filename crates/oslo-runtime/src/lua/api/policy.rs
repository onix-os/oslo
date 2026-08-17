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
            Err(LuaError::new(replace_with_run("os.execute")))
        }),
    );
    host.set_field(
        &["io", "popen"],
        native("io.popen", |_, _| {
            Err(LuaError::new(replace_with_run("io.popen")))
        }),
    );
    host.set_field(&["os", "setlocale"], setlocale());
    guard_require(host);
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

/// Make `require` notice a module that is already loading, instead of recursing until the stack
/// runs out.
///
/// **A module that requires itself took the VM down.** `package.loaded[name]` is only written
/// *after* a loader returns, so the VM's `require` had nothing to check on the way in: a file
/// containing `require` of its own name re-entered the search and kept re-entering it, and what
/// came back was `stack overflow` — a message about the machine, from a mistake in one line of
/// Lua. A cycle across two modules did the same.
///
/// Lua answers `loop or previous error in loading module 'x'`, which names the thing that is
/// wrong; this puts that answer back by keeping the in-progress set the VM does not.
///
/// **Cleared however the load ends.** A marker left behind by a *failed* load would report a loop
/// on the next attempt, turning one broken module into a module that can never be loaded again —
/// which is why `pcall` is here rather than a plain call.
///
/// Written in Lua because it wraps a function the VM installed: reaching it from a native callback
/// would mean calling back into the interpreter to do what one line of Lua already does.
fn guard_require(host: &dyn Host) {
    let source = r#"
        local plain = require
        local loading = {}
        require = function(name, ...)
            local key = tostring(name)
            if loading[key] then
                error("loop or previous error in loading module '" .. key .. "'", 2)
            end
            loading[key] = true
            local answer = table.pack(pcall(plain, name, ...))
            loading[key] = nil
            if not answer[1] then
                -- Level 0: the error already carries its own position, and adding this
                -- wrapper's would name a file the person did not write.
                error(answer[2], 0)
            end
            -- `require` answers the module *and* the loader data, so both are passed on.
            return table.unpack(answer, 2, answer.n)
        end
    "#;
    if let Err(e) = host.eval(source, "=oslo.require") {
        oslo_base::messages::error("oslo require", e.to_string());
    }
}

/// The only locale a static binary has, and the name C gives it.
const C_LOCALE: &str = "C";

/// The categories `setlocale` accepts, from the Lua manual.
const CATEGORIES: [&str; 6] = ["all", "collate", "ctype", "monetary", "numeric", "time"];

/// `os.setlocale([locale [, category]])` — the one standard name the VM does not define.
///
/// **A gap rather than a refusal**, and the only one on the standard surface: left `nil`, the first
/// use of it is `attempt to call a nil value` and the reader goes looking for a typo that is not
/// there. It is also the one name here that is genuinely *answerable* — this binary is static and
/// musl-linked, so `C` is not merely the current locale, it is the only one there is.
///
/// So the answers are the true ones rather than a stub's: asking gives `"C"`, asking for `"C"`,
/// `"POSIX"` or `""` succeeds and gives `"C"`, and asking for anything else gives `nil` — which is
/// exactly what Lua specifies for a locale the implementation cannot honour, and exactly what
/// stock Lua answers on a machine that lacks the locale asked for.
fn setlocale() -> Value {
    native("os.setlocale", |_, args| {
        // Checked even though the answer does not depend on it: a typo in the category is a
        // mistake Lua reports, and accepting it here would be this file's own silent acceptance.
        match args.get(1) {
            None | Some(Value::Nil) => {}
            Some(Value::Str(category)) if CATEGORIES.contains(&category.as_ref()) => {}
            Some(Value::Str(category)) => {
                return Err(LuaError::new(format!(
                    "bad argument #2 to 'setlocale' (invalid option '{category}')"
                )));
            }
            Some(other) => {
                return Err(LuaError::new(format!(
                    "bad argument #2 to 'setlocale' (string expected, got {})",
                    other.type_name()
                )));
            }
        }
        Ok(vec![match args.first() {
            // `nil`, or nothing at all, asks which locale is in force rather than setting one.
            None | Some(Value::Nil) => Value::str(C_LOCALE),
            // `""` is "the native locale", which on this binary is the same one.
            Some(Value::Str(asked)) if matches!(asked.as_ref(), "C" | "POSIX" | "") => {
                Value::str(C_LOCALE)
            }
            _ => Value::Nil,
        }])
    })
}

/// The refusal `os.execute` and `io.popen` share, with the name of the one that was called.
///
/// **Named, because the two of them said the same nameless sentence.** "this runs its argument
/// through /bin/sh" told a reader what the problem was and not which call had it, and a traceback
/// pointing at a line that uses both is exactly the case where that matters. `os.tmpname` had
/// named itself since it was written; these two now do too.
fn replace_with_run(name: &str) -> String {
    format!(
        "{name} runs its argument through /bin/sh — another shell, from inside this one, and \
         absent entirely where oslo is the only one installed. Use oslo.run{{\"cmd\", \"arg\"}}, \
         which takes an argv and needs no shell to read it."
    )
}
