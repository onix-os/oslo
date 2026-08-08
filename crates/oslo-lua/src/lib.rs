//! oslo's own Lua evaluator.
//!
//! `full_moon` parses; everything here runs. That is the same relationship `brush-parser` has to
//! the shell, and it is the point: **one Rust core, two interfaces.** A builtin lives in
//! `crate::env::builtins`, and both `ls -la` typed as shell and `sh.ls("-la")` written in Lua
//! reach the same function. Nothing is implemented twice.
//!
//! ## What this is not
//!
//! It is not a complete Lua 5.4. Coroutines, weak tables, `__gc` and `__close` are out, and so is
//! a tracing garbage collector — reference cycles leak. That is a recorded decision, not an
//! oversight; see PLAN.md.
//!
//! What matters is *how* the missing parts fail. Everything unimplemented is **present and
//! erroring**, never absent:
//!
//! ```text
//! oslo: coroutine.create is not implemented in oslo's Lua
//!   at /usr/share/lua/dkjson.lua:412
//! ```
//!
//! If `coroutine` were simply `nil`, the script would fail with `attempt to index a nil value`
//! and the reader would have to work out why. A partial implementation that says so is far easier
//! to live with — and every such error is a concrete request for the next thing to build.

mod expr;
mod ops;
pub mod scope;
mod stdlib;
mod stmt;
pub mod value;

pub use scope::{Closure, Scope};
pub use value::{Number, Table, Value};

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A Lua error: the message, where it happened, and the call stack under it.
#[derive(Debug, Clone)]
pub struct LuaError {
    pub message: String,
    /// What the source was called — a path, or `-c`. Stamped as the error leaves the chunk.
    pub chunk: Option<String>,
    /// Source line, when the AST node carried one.
    pub line: Option<usize>,
    /// Innermost frame last, for the traceback.
    pub frames: Vec<String>,
    /// Set when this is `oslo.proc.exit(n)` rather than a failure.
    ///
    /// An exit travels as an error because unwinding is the only way out of a call that is
    /// several Lua frames deep. It is deliberately *not* catchable: `pcall` re-raises it, so
    /// `pcall(oslo.proc.exit)` ends the shell rather than reporting a caught error, which is what
    /// "never returns" has to mean.
    pub exit: Option<i32>,
}

impl LuaError {
    pub fn new(message: impl Into<String>) -> Self {
        LuaError {
            message: message.into(),
            chunk: None,
            line: None,
            frames: Vec::new(),
            exit: None,
        }
    }

    /// A request to end the shell with `status`, dressed as an error so that it unwinds.
    pub fn exit_request(status: i32) -> Self {
        LuaError {
            exit: Some(status),
            ..LuaError::new(format!("exit {status}"))
        }
    }

    /// Attach a source line, if one is not already recorded.
    ///
    /// Innermost wins: the line where the error actually happened is more useful than the line of
    /// the call that led there, and the outer frames are in `frames` anyway.
    pub fn at(mut self, line: usize) -> Self {
        self.line.get_or_insert(line);
        self
    }

    /// Record a call frame as the error unwinds.
    pub fn in_frame(mut self, frame: impl Into<String>) -> Self {
        self.frames.push(frame.into());
        self
    }

    /// Name the source this came out of, if it is not already named.
    pub fn in_chunk(mut self, chunk: impl Into<String>) -> Self {
        if self.chunk.is_none() {
            self.chunk = Some(chunk.into());
        }
        self
    }

    /// The error as Lua's own *value*: `chunk:line: message`.
    ///
    /// This is what `pcall` hands back, and scripts parse it — `message:match(":(%d+):")` to find
    /// the line is a common idiom, and `error("x")` reaching a handler as a bare `x` breaks it.
    ///
    /// A chunk recorded on the error wins over `current`. An error raised inside a `require`d
    /// module has already unwound past it by the time `pcall` renders this, so the interpreter is
    /// back on the outer chunk — and naming *that* file with the module's line number is worse
    /// than naming neither.
    pub fn value_string(&self, current: &str) -> String {
        let chunk = self.chunk.as_deref().unwrap_or(current);
        match self.line {
            Some(line) => format!("{chunk}:{line}: {}", self.message),
            None => self.message.clone(),
        }
    }

    /// The message a user sees, with the traceback the plan calls for.
    pub fn report(&self) -> String {
        let mut out = self.to_string();
        if !self.frames.is_empty() {
            out.push_str("\nstack traceback:");
            for frame in self.frames.iter().rev() {
                out.push_str("\n\t");
                out.push_str(frame);
            }
        }
        out
    }
}

impl std::error::Error for LuaError {}

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Lua's own `file:line: message`, with each part dropped when it is not known — a syntax
        // error has a chunk but no line, and an error raised before any chunk was named has
        // neither.
        match (&self.chunk, self.line) {
            (Some(chunk), Some(line)) => write!(f, "{chunk}:{line}: {}", self.message),
            (Some(chunk), None) => write!(f, "{chunk}: {}", self.message),
            (None, Some(line)) => write!(f, "{line}: {}", self.message),
            (None, None) => f.write_str(&self.message),
        }
    }
}

pub type LuaResult<T> = Result<T, LuaError>;

/// How a statement finished.
///
/// A separate type rather than an error because `break` and `return` are ordinary control flow —
/// folding them into `LuaError` is how a `return` inside a `pcall` ends up looking like a failure.
pub enum Flow {
    Normal,
    Break,
    Return(Vec<Value>),
}

/// The interpreter: global state plus whatever the host attaches to it.
///
/// Every method takes `&self`, with the mutable parts behind `Cell`/`RefCell`. That is not a
/// stylistic choice — it is what makes the two interfaces re-entrant with each other. A Lua
/// script calls `oslo.proc.exec("build")`, the shell runs `build`, and `build` turns out to be a
/// builtin the same script registered with `oslo.register_builtin`: control has to come back into
/// the interpreter that is still part-way through the outer call. With `&mut self` that second
/// entry is unreachable — the borrow is already out — and the honest workarounds are a raw
/// pointer or a second interpreter that cannot see the first one's globals.
/// The shell's variables, as seen from Lua.
///
/// Implemented by the host so that the evaluator can stay independent of `Environment`: running
/// Lua with no shell attached — a unit test, `oslo --lua-script` before startup — simply has no
/// host, and globals behave exactly as Lua's own.
pub trait Globals {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str);
    fn unset(&self, name: &str);
}

pub struct Interp {
    pub globals: Rc<RefCell<Table>>,
    /// The shell's variables, when a shell is attached. See [`Interp::set_script_global`].
    host: RefCell<Option<Rc<dyn Globals>>>,
    /// Chunk name used in diagnostics — a path, or `-c`.
    chunk: RefCell<String>,
    /// The line of the statement currently running.
    ///
    /// Tracked on the interpreter rather than threaded through every function because that is the
    /// only place a *native* function can read it from: `error("x")` is Rust code with no view of
    /// the AST, and it still has to produce `script.lua:12: x`.
    line: Cell<usize>,
    /// Call depth, to turn runaway recursion into an error rather than a stack overflow that
    /// aborts the whole shell.
    depth: Cell<usize>,
    /// Varargs of the function currently running, for `...`.
    varargs: RefCell<Vec<Value>>,
}

/// Deep enough for any real script, shallow enough to unwind before the Rust stack gives out.
///
/// A shell may not abort. Without this, `local function f() return f() end f()` takes the process
/// down with SIGSEGV and no diagnostic — this evaluator has no tail-call optimisation, so even
/// correct Lua recurses on the Rust stack.
///
/// 200 is the same ceiling real Lua puts on nested C calls. It holds because oslo runs on a stack
/// it reserved itself ([`crate::INTERPRETER_STACK`]) rather than on whatever `ulimit -s` gave it;
/// a level of Lua recursion costs several kilobytes of Rust frames in an unoptimised build, so
/// against a 1 MiB stack the honest limit would be nearer fifty.
const MAX_DEPTH: usize = 200;

impl Interp {
    pub fn new(chunk: impl Into<String>) -> Self {
        let interp = Interp {
            globals: Rc::new(RefCell::new(Table::new())),
            host: RefCell::new(None),
            chunk: RefCell::new(chunk.into()),
            line: Cell::new(0),
            depth: Cell::new(0),
            varargs: RefCell::new(Vec::new()),
        };
        stdlib::install(&interp);
        interp
    }

    /// What diagnostics call the source currently running.
    pub fn chunk_name(&self) -> String {
        self.chunk.borrow().clone()
    }

    /// Name the source about to run, so its errors point at the right file.
    pub fn set_chunk(&self, name: impl Into<String>) {
        *self.chunk.borrow_mut() = name.into();
    }

    /// The line currently executing.
    pub fn line(&self) -> usize {
        self.line.get()
    }

    /// Record the line about to execute.
    pub fn set_line(&self, line: usize) {
        self.line.set(line);
    }

    /// The varargs of the function currently running, for `...`.
    pub fn varargs(&self) -> Vec<Value> {
        self.varargs.borrow().clone()
    }

    /// Give the chunk itself a `...`, which is how a script reads its own arguments.
    pub fn set_varargs(&self, values: Vec<Value>) {
        *self.varargs.borrow_mut() = values;
    }

    /// Replace the varargs for the duration of a call, answering what was there before.
    fn swap_varargs(&self, values: Vec<Value>) -> Vec<Value> {
        self.varargs.replace(values)
    }

    /// Attach the shell's variables, making Lua globals and shell variables one namespace.
    pub fn set_host(&self, host: Rc<dyn Globals>) {
        *self.host.borrow_mut() = Some(host);
    }

    /// Read a global.
    ///
    /// `_G` first, the shell second. The order is the whole safety argument: a shell script that
    /// sets `type=deploy` or `print=/usr/bin/lpr` must not break `type()` and `print()` in Lua,
    /// and putting the standard library first means it cannot.
    pub fn global(&self, name: &str) -> Value {
        let own = self.globals.borrow().get(&Value::str(name));
        if !matches!(own, Value::Nil) {
            return own;
        }
        match self.host.borrow().as_ref().and_then(|h| h.get(name)) {
            Some(value) => Value::str(value),
            None => Value::Nil,
        }
    }

    /// Write a global from the host: always `_G`, never the shell.
    ///
    /// This is how the standard library is installed. Routing `print` through the shell's
    /// variables would export a function as a string and make `env` unreadable.
    pub fn set_global(&self, name: &str, value: Value) {
        self.globals.borrow_mut().set(Value::str(name), value);
    }

    /// Write a global from a *script*, which is where the two namespaces meet.
    ///
    /// A string lands in the shell, so `name = "world"` in Lua is `$name` in shell on the next
    /// line. Anything else — a table, a function — stays in `_G`, because a shell variable can
    /// only hold a string and flattening one to `table: 0x55f…` loses it.
    ///
    /// Each name lives in exactly one of the two. Writing a string clears any `_G` entry and
    /// writing a non-string clears the shell variable, so a name that changes type moves rather
    /// than leaving a stale copy for the *other* lookup order to find later.
    pub fn set_script_global(&self, name: &str, value: Value) {
        let host = self.host.borrow();
        let Some(host) = host.as_ref() else {
            return self.set_global(name, value);
        };
        match &value {
            Value::Str(text) => {
                self.globals.borrow_mut().set(Value::str(name), Value::Nil);
                host.set(name, text);
            }
            // `x = nil` removes the name from both homes, which is what "unset" means in each.
            Value::Nil => {
                self.globals.borrow_mut().set(Value::str(name), Value::Nil);
                host.unset(name);
            }
            _ => {
                host.unset(name);
                self.globals.borrow_mut().set(Value::str(name), value);
            }
        }
    }

    /// Run a parsed chunk, returning whatever it returned.
    pub fn run_ast(&self, ast: &full_moon::ast::Ast) -> LuaResult<Vec<Value>> {
        let scope = Scope::root();
        let outcome =
            stmt::exec_block(self, ast.nodes(), &scope).map_err(|e| e.at(self.line.get()));
        match outcome? {
            Flow::Return(values) => Ok(values),
            _ => Ok(Vec::new()),
        }
    }

    /// Call any callable with `args`.
    pub fn call(&self, callee: &Value, args: Vec<Value>) -> LuaResult<Vec<Value>> {
        let Value::Function(f) = callee else {
            // Before giving up, Lua consults `__call`, which is what makes callable tables work.
            if let Value::Table(t) = callee
                && let Some(handler) = ops::metamethod(callee, "__call")
            {
                let mut full = vec![Value::Table(Rc::clone(t))];
                full.extend(args);
                return self.call(&handler, full);
            }
            return Err(LuaError::new(format!(
                "attempt to call a {} value",
                callee.type_name()
            )));
        };

        self.depth.set(self.depth.get() + 1);
        if self.depth.get() > MAX_DEPTH {
            self.depth.set(self.depth.get() - 1);
            return Err(LuaError::new("stack overflow: too many nested calls"));
        }
        let caller_line = self.line.get();
        let result = match &**f {
            value::Function::Native { call, name } => {
                call(self, args).map_err(|e| e.in_frame(format!("in function '{name}'")))
            }
            value::Function::Lua(closure) => self.call_closure(closure, args),
        };
        // Stamped before the caller's line is restored, so the error carries the line it actually
        // happened on rather than the line of the outermost call that led there.
        let result = result.map_err(|e| e.at(self.line.get()));
        self.line.set(caller_line);
        self.depth.set(self.depth.get() - 1);
        result
    }

    /// Bind arguments into a fresh scope and run a Lua function's body.
    fn call_closure(&self, closure: &Closure, args: Vec<Value>) -> LuaResult<Vec<Value>> {
        let scope = Scope::child(&closure.captured);
        let mut args = args.into_iter();
        for name in &closure.params {
            // Missing arguments are nil, never an error — Lua has no arity checking, and scripts
            // rely on it for optional parameters.
            scope.declare(Rc::clone(name), args.next().unwrap_or(Value::Nil));
        }

        let saved = self.swap_varargs(if closure.varargs {
            args.collect()
        } else {
            Vec::new()
        });

        let body = Rc::clone(&closure.body);
        let outcome = stmt::exec_block(self, body.block(), &scope);
        self.swap_varargs(saved);

        match outcome? {
            Flow::Return(values) => Ok(values),
            _ => Ok(Vec::new()),
        }
    }
}

/// Parse and run `source`, the whole front-to-back path.
pub fn run(source: &str, chunk: &str) -> LuaResult<Vec<Value>> {
    let ast = parse(source)?;
    let interp = Interp::new(chunk);
    interp.run_ast(&ast)
}

/// Parse `source`, turning full_moon's error list into one diagnostic.
pub fn parse(source: &str) -> LuaResult<full_moon::ast::Ast> {
    full_moon::parse(source).map_err(|errors| {
        let detail = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        LuaError::new(format!("syntax error: {detail}"))
    })
}

/// Whether `source` is a complete chunk, or merely unfinished.
///
/// This is what the prompt needs in order to decide between running a line and asking for another.
/// Real Lua answers it by string-matching `<eof>` in the error message, because that is all its C
/// API exposes; having our own parser means asking the parser directly.
pub fn is_complete(source: &str) -> bool {
    match full_moon::parse(source) {
        Ok(_) => true,
        // Every error pointing at the very end of the input means the chunk was cut off mid
        // construct — `if true then` and `local x = {` both land there. An error anywhere earlier
        // is a genuine mistake, and waiting for more input would only hide it: `x = = 2` never
        // becomes valid however much the user types after it.
        Err(errors) => !errors
            .iter()
            .all(|e| e.range().1.bytes() >= source.trim_end().len()),
    }
}
