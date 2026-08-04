//! Owning the Lua interpreter, and the bridge back from the shell into it.

use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::lua::context::Context;
use crate::lua::eval::value::Value;
use crate::lua::eval::{self, Interp, LuaResult};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

/// Values the host holds on a script's behalf.
///
/// The prompt function and every `oslo.register_builtin` callback live here rather than in a Lua
/// global, so a script cannot overwrite one by choosing an unlucky variable name — and so that
/// `pairs(_G)` does not walk the shell's internals.
pub(crate) type Registry = Rc<RefCell<HashMap<String, Value>>>;

/// Registry key under which `oslo.ui.prompt` stores its function.
pub(crate) const PROMPT_KEY: &str = "prompt";

/// Prefix for the registry keys holding builtin callbacks.
pub(crate) const BUILTIN_KEY_PREFIX: &str = "builtin:";

thread_local! {
    /// The interpreter and registry a builtin registered on this thread should call back into.
    ///
    /// Per-thread because the interpreter is: a builtin invoked from a thread that never ran
    /// [`LuaEngine::setup_bindings`] finds nothing here and says so, rather than reaching into
    /// another thread's state.
    static ACTIVE: RefCell<Option<(Rc<Interp>, Registry)>> = const { RefCell::new(None) };
}

/// The interpreter on this thread, if there is one, as a handle that can outlive the borrow.
pub fn interpreter_handle() -> Option<Rc<Interp>> {
    ACTIVE.with(|slot| slot.borrow().as_ref().map(|(interp, _)| Rc::clone(interp)))
}

/// Borrow the interpreter on this thread, if there is one.
///
/// The same reach-back the hooks use, for callers that need more than one call against the same
/// interpreter — a filter evaluating one parsed expression against every row, for instance.
pub fn with_interpreter<T>(f: impl FnOnce(&Interp) -> T) -> Option<T> {
    let (interp, _) = ACTIVE.with(|slot| slot.borrow().clone())?;
    Some(f(&interp))
}

/// Call a Lua value with whatever interpreter is on this thread.
///
/// The same reach-back `ask_hook_here` uses, for callers that hold a function rather than a hook
/// name — a key binding written in the config, for instance, which the editor holds directly.
pub fn call_here(f: &Value, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let Some((interp, _)) = ACTIVE.with(|slot| slot.borrow().clone()) else {
        return Err(eval::LuaError::new(
            "no Lua interpreter on this thread".to_string(),
        ));
    };
    interp.call(f, args)
}

/// Fire an answering hook using whatever interpreter is on this thread.
///
/// The executor is a long way from the place the engine is owned — `run_simple` has no route back
/// to `repl` — but the interpreter is already parked on this thread for exactly this kind of
/// reach-back. `None` when there is no interpreter (a non-interactive shell, a script) or when no
/// handler answered, and the caller then does what it did before.
pub fn ask_hook_here(name: &str, args: Vec<Value>) -> Option<i32> {
    let (interp, registry) = ACTIVE.with(|slot| slot.borrow().clone())?;
    ask_hook_on(&interp, &registry, name, args)
}

fn ask_hook_on(interp: &Interp, registry: &Registry, name: &str, args: Vec<Value>) -> Option<i32> {
    for handler in crate::lua::api::hook_handlers(registry, name) {
        match interp.call(&handler, args.clone()) {
            Ok(values) => {
                if let Some(Value::Number(n)) = values.first() {
                    return n.as_int().map(|i| i as i32);
                }
            }
            // Reported and skipped, as with any other hook: one broken handler must not stop the
            // others, and must not turn a missing command into a silent success.
            Err(e) => eprintln!("oslo: {name} hook: {e}"),
        }
    }
    None
}

/// Take the shell state for the duration of one `oslo.*` call.
///
/// `try_lock`, not `lock`: the interpreter runs *inside* the evaluator when a Lua-registered
/// builtin executes, and the evaluator is already holding this mutex. `lock` there is a hang —
/// the shell stops responding with no output at all. A Lua error instead is recoverable and says
/// what happened.
pub(crate) fn borrow_env(env: &Mutex<Environment>) -> LuaResult<MutexGuard<'_, Environment>> {
    env.try_lock().map_err(|_| {
        eval::LuaError::new(
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
fn status_from_lua(value: Option<&Value>) -> i32 {
    match value {
        None | Some(Value::Nil) | Some(Value::Bool(true)) => 0,
        Some(Value::Bool(false)) => 1,
        Some(Value::Number(n)) => n.as_float() as i32,
        Some(Value::Str(s)) => s.parse().unwrap_or(0),
        Some(_) => 0,
    }
}

/// Run the Lua callback registered for `name` with `args` as its argument list.
///
/// Returns an exit status in every case, because a builtin that cannot run is still a command
/// that ran: a Lua error becomes a diagnostic on stderr plus status 1, the same shape a builtin
/// written in Rust uses to report a bad invocation. `args[0]` is the builtin's own name, so the
/// callback sees the same argv a native builtin does.
pub(crate) fn call_lua_builtin(name: &str, args: &[String]) -> i32 {
    let Some((interp, registry)) = ACTIVE.with(|slot| slot.borrow().clone()) else {
        eprintln!("oslo: {}: no Lua interpreter on this thread", name);
        return 127;
    };
    let key = format!("{}{}", BUILTIN_KEY_PREFIX, name);
    let Some(callback) = registry.borrow().get(&key).cloned() else {
        eprintln!("oslo: {}: no Lua callback registered", name);
        return 127;
    };

    // Argv reaches the callback as one table, which is the `function(argv)` shape the API has
    // always documented.
    let mut argv = eval::Table::new();
    for (i, arg) in args.iter().enumerate() {
        argv.set(Value::int(i as i64 + 1), Value::str(arg));
    }
    match interp.call(&callback, vec![Value::table(argv)]) {
        Ok(values) => status_from_lua(values.first()),
        Err(e) => {
            eprintln!("oslo: {}: {}", name, e);
            1
        }
    }
}

/// The shell's variables, as the evaluator's global namespace.
///
/// This is what makes `export one=two` in shell readable as `one` in Lua on the next line, and
/// `name = "world"` in Lua readable as `$name` in shell. One namespace, two spellings.
///
/// A busy lock is answered as "not set" rather than by blocking. The only way to reach here with
/// the mutex held is from inside a Lua-registered builtin, which is already running *because* the
/// evaluator holds it — blocking would be a deadlock with no output at all, where a nil is a
/// wrong answer the reader can see.
struct ShellGlobals {
    env: Arc<Mutex<Environment>>,
}

impl eval::Globals for ShellGlobals {
    fn get(&self, name: &str) -> Option<String> {
        self.env.try_lock().ok()?.get_param(name)
    }

    fn set(&self, name: &str, value: &str) {
        if let Ok(mut env) = self.env.try_lock() {
            env.set_var(name, value, true);
        }
    }

    fn unset(&self, name: &str) {
        if let Ok(mut env) = self.env.try_lock() {
            env.unset_var(name);
        }
    }
}

pub struct LuaEngine {
    /// `Rc` because the shell reaches back in through [`ACTIVE`] while a call is still running.
    interp: Rc<Interp>,
    registry: Registry,
    /// Arguments passed to the chunk as `...`; see [`LuaEngine::set_script_args`].
    script_args: RefCell<Vec<Value>>,
}

impl Default for LuaEngine {
    fn default() -> Self {
        Self::new().expect("Failed to initialize Lua engine")
    }
}

impl LuaEngine {
    pub fn new() -> Result<Self> {
        Ok(Self {
            interp: Rc::new(Interp::new("=(oslo lua)")),
            registry: Rc::new(RefCell::new(HashMap::new())),
            script_args: RefCell::new(Vec::new()),
        })
    }

    pub fn setup_bindings(&self, env: Arc<Mutex<Environment>>) -> Result<()> {
        // Published before anything can be registered, so a builtin registered by the very
        // script that is being loaded can already find its way back here.
        ACTIVE.with(|slot| {
            *slot.borrow_mut() = Some((Rc::clone(&self.interp), Rc::clone(&self.registry)))
        });
        self.interp.set_host(Rc::new(ShellGlobals {
            env: Arc::clone(&env),
        }));
        crate::lua::api::install(&self.interp, &self.registry, env);
        Ok(())
    }

    pub fn eval_script(&self, script: &str) -> Result<()> {
        self.run(script, "(oslo lua)")
    }

    pub fn load_file(&self, path: &str) -> Result<()> {
        let content = std::fs::read_to_string(path)?;
        self.eval_as(&content, path)
    }

    /// Run Lua source under `name`.
    ///
    /// Naming the chunk is what makes a traceback out of `init.lua` point at the user's script,
    /// and it is also what `pcall` hands the script: `error("x")` on line 12 is caught as
    /// `init.lua:12: x`, which is the form Lua code parses with `message:match(":(%d+):")`.
    pub fn eval_as(&self, source: &str, name: &str) -> Result<()> {
        self.run(source, name)
    }

    /// Publish a script's arguments the way every Lua interpreter does.
    ///
    /// `arg[0]` is the script, `arg[1..n]` its arguments — and the same list becomes the chunk's
    /// `...`. Without this a Lua program could not read its own argv at all: `oslo build.lua
    /// release` gave the script `arg == nil`, which makes Lua a configuration language rather
    /// than a scripting one.
    ///
    /// `arg[-1]` is the interpreter, matching `lua`'s own convention, so a script that
    /// re-executes itself can find the shell it is running under.
    pub fn set_script_args(&self, script: &str, args: &[String]) -> Result<()> {
        let mut table = eval::Table::new();
        table.set(
            Value::int(-1),
            Value::str(
                std::env::current_exe()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| "oslo".to_string()),
            ),
        );
        table.set(Value::int(0), Value::str(script));
        for (i, value) in args.iter().enumerate() {
            table.set(Value::int(i as i64 + 1), Value::str(value));
        }
        self.interp.set_global("arg", Value::table(table));
        self.script_args
            .replace(args.iter().map(Value::str).collect());
        Ok(())
    }

    fn run(&self, source: &str, name: &str) -> Result<()> {
        // Both failures are named here: a chunk that does not parse and one that fails part-way
        // through are equally useless as `Lua error: attempt to call a nil value` with no file.
        let ast = eval::parse(source).map_err(|e| ShellError::Lua(e.in_chunk(name)))?;
        self.interp.set_chunk(name);
        self.interp.set_varargs(self.script_args.borrow().clone());
        self.interp
            .run_ast(&ast)
            .map_err(|e| ShellError::Lua(e.in_chunk(name)))?;
        Ok(())
    }

    /// Run everything attached to a hook, in the order it was attached.
    ///
    /// A handler that fails is reported and the rest still run. One broken `precmd` silently
    /// disabling every other one — or, worse, stopping the command that was about to run — is
    /// how a config file becomes impossible to debug.
    pub fn fire_hook(&self, name: &str, args: Vec<Value>) {
        for handler in crate::lua::api::hook_handlers(&self.registry, name) {
            if let Err(e) = self.interp.call(&handler, args.clone()) {
                eprintln!("oslo: {name} hook: {e}");
            }
        }
    }

    /// Fire a hook that can *answer*, returning the first status a handler gave.
    ///
    /// [`fire_hook`](Self::fire_hook) is for telling the config something happened. This is for
    /// asking it a question — `command-not-found` is one: a handler that installs the package and
    /// runs the command has a status to report, and one that only prints advice has none.
    ///
    /// The first handler to return a number wins and the rest are skipped, which is what makes a
    /// chain of handlers behave: the one that resolved the situation ends it.
    pub fn ask_hook(&self, name: &str, args: Vec<Value>) -> Option<i32> {
        ask_hook_on(&self.interp, &self.registry, name, args)
    }

    /// A string argument for a hook, so callers do not need the value type.
    pub fn hook_arg(text: &str) -> Value {
        Value::str(text)
    }

    /// A number argument for a hook.
    pub fn hook_status(status: i32) -> Value {
        Value::int(status as i64)
    }

    /// Render one of the prompts, or `None` when the config set none.
    ///
    /// Three shapes, in the order a config is likely to reach for them: a **string** used as
    /// written, so a prompt that never changes need not be a closure; a **function** called for its
    /// return value; and a **list of segments**, which is the one that can be measured, styled and
    /// degraded piece by piece — see the `segment` module for the shape of one.
    pub fn render(&self, key: &str) -> Option<String> {
        self.render_with(key, &Context::default())
    }

    /// As [`render`](Self::render), with the facts a segment's `render(ctx)` is given.
    pub fn render_with(&self, key: &str, ctx: &Context) -> Option<String> {
        let value = self.registry.borrow().get(key).cloned()?;
        if let Value::Str(text) = &value {
            return Some(text.to_string());
        }
        if crate::lua::api::segment::is_segment_list(&value) {
            return self.render_segments(&value, ctx);
        }
        // A prompt produced by another program — starship, hexe, anything that prints one.
        if let Some(spec) = crate::lua::api::external::spec_of(&value) {
            return crate::lua::api::external::render(&spec, ctx);
        }
        // **The facts go in as the argument.** A segment list and an external prompt were handed
        // the context above; a plain function was called with nothing, so `oslo.prompt.left =
        // function(p) return p.cwd end` — the shape the documentation shows and the obvious one to
        // write — saw `p` as nil and raised. Every prompt key is a function of the same facts, so
        // there is no reason for the three shapes to disagree.
        //
        // Zero-argument functions are unaffected: Lua discards arguments a function does not name.
        match self.interp.call(&value, vec![ctx.to_lua()]) {
            Ok(values) => match values.first() {
                Some(Value::Str(s)) => Some(s.to_string()),
                Some(Value::Number(n)) => Some(n.to_string()),
                _ => None,
            },
            Err(e) => {
                // Reported rather than swallowed: a prompt function that raises leaves the shell
                // silently drawing its default, which looks exactly like the config not loading.
                eprintln!("oslo: {key}: {e}");
                None
            }
        }
    }

    /// Read `oslo.theme` as it stands, and what was wrong with it.
    pub fn read_theme(&self) -> (crate::interactive::theme::Theme, Vec<String>) {
        let oslo = self.interp.global("oslo");
        let theme = match &oslo {
            Value::Table(table) => table.borrow().get(&Value::str("theme")),
            _ => Value::Nil,
        };
        crate::interactive::theme::read_lua_theme(&theme)
    }

    /// Read `oslo.completion`, `oslo.suggest` and `oslo.history` as they stand.
    pub fn read_settings(&self) -> (crate::interactive::settings::Settings, Vec<String>) {
        crate::interactive::settings::read_lua_settings(&self.interp.global("oslo"))
    }

    /// Install `oslo.completion.columns` as the dropdown's column provider.
    ///
    /// Called after the config has run, for the same reason the theme is read then rather than
    /// pushed in as it goes: a config may set the function, change its mind, and set another.
    pub fn install_column_provider(&self) {
        crate::lua::columns::install(&self.interp);
    }

    /// Render a list of segments into one string, dropping the least important until it fits.
    fn render_segments(&self, list: &Value, ctx: &Context) -> Option<String> {
        use crate::lua::api::segment;
        let Value::Table(table) = list else {
            return None;
        };
        let count = table.borrow().length();
        let ctx_value = ctx.to_lua();
        let mut pieces = Vec::new();
        for i in 1..=count {
            let seg = table.borrow().get(&Value::int(i));
            let (name, priority) = segment::describe(&seg);
            let Some(render) = segment::render_fn(&seg) else {
                continue;
            };
            let produced = match self.interp.call(&render, vec![ctx_value.clone()]) {
                Ok(values) => values.first().cloned().unwrap_or(Value::Nil),
                Err(e) => {
                    // Named, because with several segments "the prompt failed" does not say which.
                    eprintln!("oslo: prompt: segment '{name}': {e}");
                    continue;
                }
            };
            let text = segment::spans_to_text(&produced, &|body, style| {
                crate::lua::api::prompt::style_named(style)
                    .paint(body, crate::interactive::theme::depth())
            });
            if text.is_empty() {
                continue;
            }
            let width = crate::interactive::prompt::printed_width(&text);
            pieces.push(segment::Rendered {
                name,
                priority,
                text,
                width,
            });
        }
        // Half the terminal at most: a prompt wider than that leaves no room to type, which is the
        // thing the prompt exists to serve.
        let budget = ctx.cols.max(20) / 2;
        let kept = segment::fit(pieces, budget);
        Some(kept.into_iter().map(|p| p.text).collect())
    }

    /// Install `oslo.completion.for_command`, the per-command completion hook.
    pub fn install_command_completer(&self) {
        crate::lua::columns::install_command_completer(&self.interp);
    }

    /// The prompt function currently installed, if any.
    ///
    /// For a caller that has to put it back — a `.env.lua` may set the prompt for its directory,
    /// and leaving that directory has to restore whatever was there before. Returned as the opaque
    /// value it is; nothing outside Lua can do anything with it except hand it back.
    pub fn prompt_handler(&self) -> Option<Value> {
        self.registry.borrow().get(PROMPT_KEY).cloned()
    }

    /// Put a prompt function back, or remove it when there was none.
    pub fn restore_prompt(&self, handler: Option<Value>) {
        match handler {
            Some(handler) => {
                self.registry
                    .borrow_mut()
                    .insert(PROMPT_KEY.to_string(), handler);
            }
            None => {
                self.registry.borrow_mut().remove(PROMPT_KEY);
            }
        }
    }

    pub fn render_prompt(&self) -> Option<String> {
        let prompt = self.registry.borrow().get(PROMPT_KEY).cloned()?;
        match self.interp.call(&prompt, Vec::new()) {
            // Anything but a string is not a prompt. Rendering `nil` or a table's address into
            // the line the user types on is worse than falling back to the shell's default.
            Ok(values) => match values.first() {
                Some(Value::Str(s)) => Some(s.to_string()),
                _ => None,
            },
            Err(_) => None,
        }
    }
}

#[cfg(test)]
#[path = "engine/tests.rs"]
mod tests;
