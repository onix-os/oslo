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
pub(crate) const BUILTIN_KEY_PREFIX: &str = "oslo_builtin_";

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
pub(crate) fn borrow_env(env: &Mutex<Environment>) -> LuaResult<MutexGuard<'_, Environment>> {
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
pub(crate) fn call_lua_builtin(name: &str, args: &[String]) -> i32 {
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
    /// Arguments passed to the chunk as `...`; see [`LuaEngine::set_script_args`].
    script_args: RefCell<Vec<String>>,
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
            script_args: RefCell::new(Vec::new()),
            prompt_fn: None,
            precmd_fn: None,
            postcmd_fn: None,
            cd_fn: None,
        })
    }

    pub fn setup_bindings(&self, env: Arc<Mutex<Environment>>) -> Result<()> {
        // Published before anything can be registered, so a builtin registered by the very
        // script that is being loaded can already find its way back here.
        ACTIVE_LUA.with(|slot| *slot.borrow_mut() = Some(self.lua.clone()));
        crate::lua::api::install(&self.lua, env).map_err(ShellError::Lua)
    }

    pub fn eval_script(&self, script: &str) -> Result<()> {
        self.eval_named(script, "=(oslo lua)")
    }

    pub fn load_file(&self, path: &str) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        self.eval_as(&content, path)
    }

    /// Run Lua source under `name`.
    ///
    /// Naming the chunk is what makes a traceback out of `init.lua` — including one raised inside
    /// a builtin registered there — point at the user's script. Lua's default chunk name is the
    /// first line of the source, which for a chunk loaded from a string is the Rust call site.
    pub fn eval_as(&self, source: &str, name: &str) -> Result<()> {
        self.eval_named(source, &format!("@{}", name))
    }

    /// Publish a script's arguments the way every Lua interpreter does.
    ///
    /// `arg[0]` is the script, `arg[1..n]` its arguments — and the same list is passed to the
    /// chunk so `...` works too. Without this a Lua program could not read its own argv at all:
    /// `oslo build.lua release` gave the script `arg == nil`, which makes Lua a configuration
    /// language rather than a scripting one.
    ///
    /// `arg[-1]` is the interpreter, matching `lua`'s own convention, so a script that re-executes
    /// itself can find the shell it is running under.
    pub fn set_script_args(&self, script: &str, args: &[String]) -> Result<()> {
        let table = self.lua.create_table().map_err(ShellError::Lua)?;
        table
            .set(
                -1i32,
                std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "oslo".to_string()),
            )
            .map_err(ShellError::Lua)?;
        table.set(0i32, script).map_err(ShellError::Lua)?;
        for (i, value) in args.iter().enumerate() {
            table
                .set(i as i32 + 1, value.as_str())
                .map_err(ShellError::Lua)?;
        }
        self.lua
            .globals()
            .set("arg", table)
            .map_err(ShellError::Lua)?;
        self.script_args
            .replace(args.iter().map(String::from).collect());
        Ok(())
    }

    fn eval_named(&self, script: &str, chunk_name: &str) -> Result<()> {
        // `call` rather than `exec`, so the arguments reach the chunk's `...` as well as `arg`.
        // With no arguments set this is exactly what `exec` did.
        // `Variadic`, not `Vec`: a `Vec` converts to a single Lua *table* argument, so `...`
        // would be one table rather than the arguments themselves.
        let args = mlua::Variadic::from_iter(self.script_args.borrow().iter().cloned());
        self.lua
            .load(script)
            .set_name(chunk_name)
            .call::<()>(args)
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
