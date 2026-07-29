//! The `oslo.*` table: what an `init.lua` can reach.

use crate::env::Environment;
use crate::error::{Result, ShellError};
use mlua::prelude::*;
use std::cell::RefCell;
use std::sync::{Arc, Mutex, MutexGuard};

/// Where a Lua-registered builtin's callback is kept, keyed by builtin name.
///
/// It cannot live in the closure that ends up in the builtin registry: `LuaFunction` is tied to
/// its interpreter and is neither `Send` nor `Sync`, while the registry stores
/// `Arc<dyn Fn … + Send + Sync>` so that `Environment` stays sendable. The closure therefore
/// captures only the name and looks the callback back up through here.
const BUILTIN_KEY_PREFIX: &str = "oslo_builtin_";

thread_local! {
    /// The interpreter a builtin registered on this thread should call back into.
    ///
    /// A `Lua` handle is a cheap clone of the same state, so this is the interpreter
    /// [`LuaEngine::setup_bindings`] was called on — not a second one. Per-thread because Lua
    /// itself is per-thread; a builtin invoked from a thread that never ran `setup_bindings`
    /// finds nothing here and says so rather than reaching into another thread's state.
    static ACTIVE_LUA: RefCell<Option<Lua>> = const { RefCell::new(None) };
}

/// Take the shell state for the duration of one `oslo.*` call.
///
/// `try_lock`, not `lock`: the interpreter runs *inside* the evaluator when a Lua-registered
/// builtin executes, and the evaluator is already holding this mutex. `lock` there is a hang —
/// the shell stops responding with no output at all. A Lua error instead is recoverable and says
/// what happened.
fn borrow_env(env: &Mutex<Environment>) -> LuaResult<MutexGuard<'_, Environment>> {
    env.try_lock().map_err(|_| {
        mlua::Error::runtime(
            "oslo: shell state is busy; the oslo.* API cannot be used from inside a builtin \
             registered with oslo.register_builtin",
        )
    })
}

/// Turn whatever a Lua builtin returned into an exit status.
///
/// Modelled on how a shell reads a command's result rather than on Lua's own truthiness: no
/// return value at all is success (the common case — a builtin that just printed something),
/// `false` is failure, and a number is the status the script asked for.
fn status_from_lua(value: LuaValue) -> i32 {
    match value {
        LuaValue::Nil => 0,
        LuaValue::Boolean(true) => 0,
        LuaValue::Boolean(false) => 1,
        LuaValue::Integer(n) => n as i32,
        LuaValue::Number(n) => n as i32,
        LuaValue::String(s) => s.to_str().ok().and_then(|t| t.parse().ok()).unwrap_or(0),
        _ => 0,
    }
}

/// Run the Lua callback registered for `name` with `args` as its argument list.
///
/// Returns an exit status in every case, because a builtin that cannot run is still a command
/// that ran: a Lua error becomes a diagnostic on stderr plus status 1, the same shape a builtin
/// written in Rust uses to report a bad invocation. `args[0]` is the builtin's own name, so the
/// callback sees the same argv a native builtin does.
fn call_lua_builtin(name: &str, args: &[String]) -> i32 {
    let Some(lua) = ACTIVE_LUA.with(|slot| slot.borrow().clone()) else {
        eprintln!("oslo: {}: no Lua interpreter on this thread", name);
        return 127;
    };
    let key = format!("{}{}", BUILTIN_KEY_PREFIX, name);
    let call = || -> LuaResult<LuaValue> {
        let func: LuaFunction = lua.named_registry_value(&key)?;
        let argv = lua.create_sequence_from(args.iter().map(String::as_str))?;
        func.call::<LuaValue>(argv)
    };
    match call() {
        Ok(value) => status_from_lua(value),
        Err(err) => {
            eprintln!("oslo: {}: {}", name, err);
            1
        }
    }
}

pub struct LuaEngine {
    lua: Lua,
    pub prompt_fn: Option<LuaFunction>,
    pub precmd_fn: Option<LuaFunction>,
    pub postcmd_fn: Option<LuaFunction>,
    pub cd_fn: Option<LuaFunction>,
}

impl Default for LuaEngine {
    fn default() -> Self {
        Self::new().expect("Failed to initialize Lua engine")
    }
}

impl LuaEngine {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();

        Ok(Self {
            lua,
            prompt_fn: None,
            precmd_fn: None,
            postcmd_fn: None,
            cd_fn: None,
        })
    }

    pub fn setup_bindings(&self, env: Arc<Mutex<Environment>>) -> Result<()> {
        let globals = self.lua.globals();
        let oslo_table = self.lua.create_table()?;

        // Published before anything can be registered, so a builtin registered by the very
        // script that is being loaded can already find its way back here.
        ACTIVE_LUA.with(|slot| *slot.borrow_mut() = Some(self.lua.clone()));

        // oslo.exec(cmd_string)
        let env_exec = Arc::clone(&env);
        let exec_fn = self.lua.create_function(move |_, cmd_str: String| {
            let mut env_guard = borrow_env(&env_exec)?;
            let ast = crate::parser::parse_bash_script(&cmd_str).map_err(|e| e.into_lua_err())?;
            let status = crate::exec::eval_command_list(&mut env_guard, &ast)
                .map_err(|e| e.into_lua_err())?;
            Ok(status)
        })?;
        oslo_table.set("exec", exec_fn)?;

        // oslo.get_var(name)
        let env_get = Arc::clone(&env);
        let get_var_fn = self.lua.create_function(move |_, name: String| {
            let env_guard = borrow_env(&env_get)?;
            Ok(env_guard.get_param(&name))
        })?;
        oslo_table.set("get_var", get_var_fn)?;

        // oslo.set_var(name, val)
        let env_set = Arc::clone(&env);
        let set_var_fn = self
            .lua
            .create_function(move |_, (name, val): (String, String)| {
                let mut env_guard = borrow_env(&env_set)?;
                env_guard.set_var(&name, &val, true);
                Ok(())
            })?;
        oslo_table.set("set_var", set_var_fn)?;

        // oslo.get_pwd()
        let get_pwd_fn = self.lua.create_function(|_, ()| {
            let pwd = std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            Ok(pwd)
        })?;
        oslo_table.set("get_pwd", get_pwd_fn)?;

        // oslo.set_alias(name, target)
        let env_alias = Arc::clone(&env);
        let set_alias_fn =
            self.lua
                .create_function(move |_, (name, target): (String, String)| {
                    let mut env_guard = borrow_env(&env_alias)?;
                    env_guard.set_alias(&name, &target);
                    Ok(())
                })?;
        oslo_table.set("set_alias", set_alias_fn)?;

        // oslo.get_alias(name)
        let env_get_alias = Arc::clone(&env);
        let get_alias_fn = self.lua.create_function(move |_, name: String| {
            let env_guard = borrow_env(&env_get_alias)?;
            Ok(env_guard.get_alias(&name).map(|s| s.to_string()))
        })?;
        oslo_table.set("get_alias", get_alias_fn)?;

        // oslo.register_builtin(name, callback)
        //
        // The callback is stored and run (PLAN R9.8). Until this round it was *dropped* and the
        // stub `|_, _| Ok(0)` registered under the name instead, which is worse than doing
        // nothing: `oslo.register_builtin('ls', …)` made `ls /` print nothing and exit 0.
        let env_builtin = Arc::clone(&env);
        let register_fn =
            self.lua
                .create_function(move |lua, (name, func): (String, LuaFunction)| {
                    let name = name.trim().to_string();
                    if name.is_empty() {
                        return Err(mlua::Error::runtime(
                            "oslo.register_builtin: the builtin name must not be empty",
                        ));
                    }
                    // Registered with the interpreter first: if this fails there is no name in
                    // the shell's registry pointing at a callback that is not there.
                    lua.set_named_registry_value(&format!("{}{}", BUILTIN_KEY_PREFIX, name), func)?;
                    let mut env_guard = borrow_env(&env_builtin)?;
                    let key = name.clone();
                    env_guard.register_dynamic_builtin(&name, move |_env, args| {
                        Ok(call_lua_builtin(&key, args))
                    });
                    Ok(())
                })?;
        oslo_table.set("register_builtin", register_fn)?;

        // oslo.set_prompt(callback)
        let set_prompt_fn = self.lua.create_function(|lua, func: LuaFunction| {
            lua.set_named_registry_value("oslo_prompt_fn", func)?;
            Ok(())
        })?;
        oslo_table.set("set_prompt", set_prompt_fn)?;

        // There is deliberately no `oslo.set_right_prompt` (PLAN R9.7). It was registered, and
        // `render_right_prompt` had no caller anywhere, so a script that set one saw nothing.
        // Drawing it means writing the string at `width - len` and restoring the cursor before
        // handing the line over — and rustyline repaints from the prompt to end-of-line on every
        // keystroke, erasing it again. Advertising an API that cannot work is worse than not
        // having one; if it comes back it comes back with a line editor that supports it.

        globals.set("oslo", oslo_table)?;
        Ok(())
    }

    pub fn eval_script(&self, script: &str) -> Result<()> {
        self.eval_named(script, "=(oslo lua)")
    }

    pub fn load_file(&self, path: &str) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        // Named after the file, so a traceback out of `init.lua` — including one raised inside a
        // builtin registered there — points at the user's script. Lua's default chunk name is the
        // first line of the source, which for a chunk loaded from a string is the Rust call site.
        self.eval_named(&content, &format!("@{}", path))
    }

    fn eval_named(&self, script: &str, chunk_name: &str) -> Result<()> {
        self.lua
            .load(script)
            .set_name(chunk_name)
            .exec()
            .map_err(ShellError::Lua)
    }

    pub fn render_prompt(&self) -> Option<String> {
        if let Ok(func) = self
            .lua
            .named_registry_value::<LuaFunction>("oslo_prompt_fn")
        {
            func.call::<String>(()).ok()
        } else {
            None
        }
    }
}
