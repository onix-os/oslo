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
//! oversight; see PLAN-LUA.md.
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

use std::cell::RefCell;
use std::rc::Rc;

/// A Lua error: the message, where it happened, and the call stack under it.
#[derive(Debug, Clone)]
pub struct LuaError {
    pub message: String,
    /// Source line, when the AST node carried one.
    pub line: Option<usize>,
    /// Innermost frame last, for the traceback.
    pub frames: Vec<String>,
}

impl LuaError {
    pub fn new(message: impl Into<String>) -> Self {
        LuaError {
            message: message.into(),
            line: None,
            frames: Vec::new(),
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

    /// The message a user sees, with the traceback the plan calls for.
    pub fn report(&self, chunk: &str) -> String {
        let mut out = match self.line {
            Some(line) => format!("{chunk}:{line}: {}", self.message),
            None => format!("{chunk}: {}", self.message),
        };
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

impl std::fmt::Display for LuaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}: {}", line, self.message),
            None => f.write_str(&self.message),
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
pub struct Interp {
    pub globals: Rc<RefCell<Table>>,
    /// Chunk name used in diagnostics.
    pub chunk: String,
    /// Call depth, to turn runaway recursion into an error rather than a stack overflow that
    /// aborts the whole shell.
    depth: usize,
    /// Varargs of the function currently running, for `...`.
    pub varargs: Vec<Value>,
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
        let globals = Rc::new(RefCell::new(Table::new()));
        let mut interp = Interp {
            globals,
            chunk: chunk.into(),
            depth: 0,
            varargs: Vec::new(),
        };
        stdlib::install(&mut interp);
        interp
    }

    /// Read a global.
    pub fn global(&self, name: &str) -> Value {
        self.globals.borrow().get(&Value::str(name))
    }

    /// Write a global.
    pub fn set_global(&self, name: &str, value: Value) {
        self.globals.borrow_mut().set(Value::str(name), value);
    }

    /// Run a parsed chunk, returning whatever it returned.
    pub fn run_ast(&mut self, ast: &full_moon::ast::Ast) -> LuaResult<Vec<Value>> {
        let scope = Scope::root();
        match stmt::exec_block(self, ast.nodes(), &scope)? {
            Flow::Return(values) => Ok(values),
            _ => Ok(Vec::new()),
        }
    }

    /// Call any callable with `args`.
    pub fn call(&mut self, callee: &Value, args: Vec<Value>) -> LuaResult<Vec<Value>> {
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

        self.depth += 1;
        if self.depth > MAX_DEPTH {
            self.depth -= 1;
            return Err(LuaError::new("stack overflow: too many nested calls"));
        }
        let result = match &**f {
            value::Function::Native { call, name } => {
                call(self, args).map_err(|e| e.in_frame(format!("in function '{name}'")))
            }
            value::Function::Lua(closure) => self.call_closure(closure, args),
        };
        self.depth -= 1;
        result
    }

    /// Bind arguments into a fresh scope and run a Lua function's body.
    fn call_closure(&mut self, closure: &Closure, args: Vec<Value>) -> LuaResult<Vec<Value>> {
        let scope = Scope::child(&closure.captured);
        let mut args = args.into_iter();
        for name in &closure.params {
            // Missing arguments are nil, never an error — Lua has no arity checking, and scripts
            // rely on it for optional parameters.
            scope.declare(Rc::clone(name), args.next().unwrap_or(Value::Nil));
        }

        let saved = std::mem::take(&mut self.varargs);
        self.varargs = if closure.varargs {
            args.collect()
        } else {
            Vec::new()
        };

        let body = Rc::clone(&closure.body);
        let outcome = stmt::exec_block(self, body.block(), &scope);
        self.varargs = saved;

        match outcome? {
            Flow::Return(values) => Ok(values),
            _ => Ok(Vec::new()),
        }
    }
}

/// Parse and run `source`, the whole front-to-back path.
pub fn run(source: &str, chunk: &str) -> LuaResult<Vec<Value>> {
    let ast = parse(source)?;
    let mut interp = Interp::new(chunk);
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
