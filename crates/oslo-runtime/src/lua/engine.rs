//! Owning the Lua interpreter, and the bridge back from the shell into it.

use crate::lua::context::Context;
use oslo_base::error::{Result, ShellError};
use oslo_lua as eval;
use oslo_lua::value::Value;
use oslo_lua::{Interp, LuaResult};
use oslo_shell::env::Environment;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

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

    /// The shell state the `oslo.*` API reaches, kept so that code far from it can ask whether it
    /// is currently *held*.
    ///
    /// A hook fires from places that have no `Environment` to offer and no way to get one — the
    /// line editor, `attempt_directory`, the job reaper. Whether a handler could act depends
    /// entirely on whether this mutex is free at that moment, and this is what lets the question be
    /// asked there. See [`state_is_held`] and `queue`.
    static SHELL_STATE: RefCell<Option<Arc<Mutex<Environment>>>> = const { RefCell::new(None) };
}

/// Whether the shell is in the middle of something that has its state locked.
///
/// **The question a hook fire site has to ask.** `attempt_directory` runs with the `Environment`
/// locked and the line editor does not, so the same `fire_at_here` call is safe to run inline from
/// one and useless from the other. `false` when there is no interpreter on this thread, which is
/// the non-interactive case where nothing can be attached anyway.
///
/// One uncontended `try_lock`. The shell is single-threaded through this path, so the answer cannot
/// go stale between here and the caller acting on it.
pub(crate) fn state_is_held() -> bool {
    SHELL_STATE.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|env| env.try_lock().is_err())
    })
}

/// The interpreter on this thread, if there is one, as a handle that can outlive the borrow.
pub fn interpreter_handle() -> Option<Rc<Interp>> {
    // Published into `oslo_lua::current` as well, for callers below this crate. See there.
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

/// A value the host is holding under `key`, from the registry on this thread.
///
/// The reach-back `oslo.feature.when` needs: the predicate is registered while the config runs and
/// called from the read loop, which has no route back to the table it was stored in. `None` when
/// there is no interpreter here — a script, a non-interactive shell — which is also every case
/// where no config could have registered one.
pub fn host_value(key: &str) -> Option<Value> {
    let (_, registry) = ACTIVE.with(|slot| slot.borrow().clone())?;
    // Cloned out rather than handed back by reference: the caller runs the value it gets, and a
    // predicate that registered another one would meet its own outstanding borrow.
    registry.borrow().get(key).cloned()
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

/// Taking and releasing the shell state, which is also where deferred hooks are drained.
mod borrow;
/// The hook reach-backs, which every fire site outside this file uses.
mod hooks;
/// Running a plugin's entry file on this session's interpreter.
#[cfg(feature = "plugin")]
mod plugin;
/// Hooks held until the shell can act on them. See that module for why.
mod queue;
pub(crate) use borrow::borrow_env;
pub use hooks::{
    answer_hook_here, answer_hook_with, ask_hook_here, fire_at_here, key_hook_here,
    key_hook_watched,
};
#[cfg(feature = "plugin")]
pub(crate) use plugin::load_plugin_file;
pub use queue::drain as run_deferred_hooks;

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
        // **The moment the shell gains a way to reach a hook.** Everything below this layer fires
        // through `crate::hooks`, which knows the moments and nothing else; this is where the four
        // functions that can actually run one are handed over. Before it — a script, `sh -c`, a
        // test that never builds an engine — every hook is a relaxed load and a return.
        oslo_base::hooks::install(oslo_base::hooks::Dispatch {
            watched: crate::lua::api::hooks::watched,
            fire: hooks::fire_at_here,
            answer: hooks::answer_hook_with,
            ask: hooks::ask_hook_here,
        });
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
        // The same interpreter, published where code *below* this crate can reach it. A `where`
        // filter in a structured pipeline wants the config's interpreter so a predicate can call a
        // function the config defined, and the pipeline is part of the shell — it cannot ask
        // upward. See `oslo_lua::current`.
        oslo_lua::current::publish(Some(Rc::clone(&self.interp)));
        // The same publication, for the question "can a hook act right now". Kept beside the
        // interpreter because the two are set up and torn down together.
        SHELL_STATE.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&env)));
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
        let outcome = self
            .interp
            .run_ast(&ast)
            .map_err(|e| ShellError::Lua(e.in_chunk(name)));
        // A script is the one place with no loop to drain into. `oslo.run{"cd", d}` at the end of
        // a chunk would otherwise leave its `post-change-dir` queued for a REPL that never comes.
        // After the failure case too: a chunk that raised still ran the commands before it.
        queue::drain();
        outcome.map(|_| ())
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
        hooks::ask_hook_on(&self.interp, &self.registry, name, args)
    }

    /// Tell a hook something happened, by its index in [`crate::lua::api::hooks::HOOKS`].
    ///
    /// By index rather than by name because a name is a spelling and there are several of each:
    /// `preexec` and `pre-cmd` are one list, and firing "the one the caller typed" would run half
    /// of it. The index names the *moment*, which is the thing a fire site actually has.
    ///
    /// The watched bit is checked first, so a moment nobody attached to costs one relaxed load —
    /// which is what lets this be called from a mode change or a prompt without thinking about it.
    ///
    /// Goes through the same inline-or-deferred decision as every other fire site rather than
    /// calling the handlers directly. These callers are in the REPL loop, where the state is free
    /// and nothing is deferred — but a second dispatch path is how the two would drift apart.
    pub fn fire_at(&self, index: usize, args: Vec<Value>) {
        hooks::fire_values_here(index, args);
    }

    /// A table for a hook, from plain pairs — the shape most of them hand over.
    pub fn hook_fields(fields: &[(&str, Value)]) -> Value {
        let mut t = oslo_lua::value::Table::new();
        for (name, value) in fields {
            t.set(Value::str(*name), value.clone());
        }
        Value::table(t)
    }

    /// A string argument for a hook, so callers do not need the value type.
    pub fn hook_arg(text: &str) -> Value {
        Value::str(text)
    }

    /// What `preexec` and `precmd` are handed: the command about to run.
    ///
    /// A table rather than the bare string these used to get. The string was never enough — a hook
    /// that wants to log a command wants to know where it ran, and `oslo.sys.cwd()` asked from
    /// inside the handler answers the same thing only until the command is a `cd`.
    pub fn command_started(text: &str, cwd: &str, mode: &str) -> Value {
        let mut t = oslo_lua::value::Table::new();
        t.set(Value::str("text"), Value::str(text));
        t.set(Value::str("cwd"), Value::str(cwd));
        t.set(Value::str("mode"), Value::str(mode));
        // **Parsed only when somebody is listening.** The line is parsed again a moment later by
        // the shell either way, so this is one extra parse per command — worth nothing to a config
        // that asks for it and worth avoiding entirely for the far more common one that does not.
        // Absent rather than empty when the line does not parse; see `lua::parsed`.
        if mode == "sh"
            && oslo_base::hooks::watched(oslo_base::hooks::at::PRE_CMD)
            && let Some(commands) = crate::lua::parsed::commands_of(text)
        {
            t.set(Value::str("commands"), commands);
        }
        Value::table(t)
    }

    /// What `postexec` and `postcmd` are handed: the command that just finished.
    ///
    /// `cwd` is where it *ended*, which is what makes the pair useful across a `cd`. `duration_ms`
    /// is the same measurement the slow-command notice and the history's timing column use, so a
    /// hook and the shell cannot disagree about how long something took.
    pub fn command_finished(
        text: &str,
        cwd: &str,
        mode: &str,
        status: i32,
        duration: std::time::Duration,
    ) -> Value {
        let mut t = oslo_lua::value::Table::new();
        t.set(Value::str("text"), Value::str(text));
        t.set(Value::str("cwd"), Value::str(cwd));
        t.set(Value::str("mode"), Value::str(mode));
        t.set(Value::str("status"), Value::int(status as i64));
        t.set(Value::str("ok"), Value::Bool(status == 0));
        t.set(
            Value::str("duration_ms"),
            Value::int(duration.as_millis() as i64),
        );
        Value::table(t)
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
    /// Whether rendering `key` again is free, in the sense of not starting a process.
    ///
    /// A string, a segment list or a Lua function is answered in this process; `{ command = … }`
    /// forks and execs whatever the user named. Callers that render a prompt *speculatively* —
    /// for a mode the user may never enter — have to know the difference, because doing that to an
    /// external prompt multiplies its cost by however many speculations they make.
    pub fn prompt_is_free(&self, key: &str) -> bool {
        match self.registry.borrow().get(key) {
            Some(value) => crate::lua::api::external::spec_of(value).is_none(),
            None => true,
        }
    }

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
    pub fn read_theme(&self) -> (oslo_ui::theme::Theme, Vec<String>) {
        let oslo = self.interp.global("oslo");
        let theme = match &oslo {
            Value::Table(table) => table.borrow().get(&Value::str("theme")),
            _ => Value::Nil,
        };
        oslo_ui::theme::read_lua_theme(&theme)
    }

    /// Read `oslo.completion`, `oslo.suggest` and `oslo.history` as they stand.
    pub fn read_settings(&self) -> (oslo_ui::settings::Settings, Vec<String>) {
        oslo_ui::settings::read_lua_settings(&self.interp.global("oslo"))
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
                crate::lua::api::prompt::style_named(style).paint(body, oslo_ui::theme::depth())
            });
            if text.is_empty() {
                continue;
            }
            let width = oslo_ui::prompt::printed_width(&text);
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
