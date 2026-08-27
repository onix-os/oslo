//! The failure shape Lua code and the shell agree on.
//!
//! **Here rather than in an engine, because it outlives every engine.** Two hundred registered
//! callables build one of these, and none of them can see a VM: they take shell values and answer
//! shell values. Keeping the error next to [`Value`](super::Value) is what lets the whole binding
//! surface compile without a Lua implementation in scope, so swapping the engine underneath is a
//! change to one crate rather than to seventy files.
//!
//! It is deliberately plain data — a message and a position. Nothing here borrows from a garbage
//! collector or an interpreter, so it can be stored, sent through a channel, and rendered long
//! after the call that raised it has gone.
//!
//! **No traceback, because the VM does not have one to give.** There used to be a `frames` vector
//! and an `in_frame` to push onto it, and a `report` that printed a `stack traceback:` under the
//! message. Nothing ever called `in_frame`: luna reports one position, not a call stack, so the
//! vector was empty at every error and the traceback branch never ran. What a reader got was the
//! same `chunk:line: message` either way, from code that promised more.

/// A Lua error: the message and where it happened.
#[derive(Debug, Clone)]
pub struct LuaError {
    pub message: String,
    /// What the source was called — a path, or `-c`. Stamped as the error leaves the chunk.
    pub chunk: Option<String>,
    /// Source line, when the failure carried one.
    pub line: Option<usize>,
    /// Set when the error value is the message and nothing else — no `chunk:line:` in front.
    ///
    /// **Two callers, and both are the language's own rule.** `error(message, 0)` says level 0,
    /// which is Lua's spelling of "do not add position information", and `assert(false, message)`
    /// raises the message as the error *object* rather than through `error`, so it never had any.
    /// Both used to arrive at a handler wearing a file and a line they had asked not to wear —
    /// which matters, because the idiom for reading one is `message:match(":(%d+):")` and a
    /// message that answers it when it should not is worse than one that never does.
    pub bare: bool,
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
            bare: false,
            exit: None,
        }
    }

    /// The message alone, with no position ever attached. See the `bare` field.
    pub fn without_position(message: impl Into<String>) -> Self {
        LuaError {
            bare: true,
            ..LuaError::new(message)
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
    /// the call that led there.
    pub fn at(mut self, line: usize) -> Self {
        if self.bare {
            return self;
        }
        self.line.get_or_insert(line);
        self
    }

    /// Name the source this came out of, if it is not already named.
    ///
    /// **Respects `bare` exactly as [`Self::at`] does**, and for the same reason: `bare` means the
    /// position was refused, and a chunk is half a position. `error("x", 0)` asks Lua for no
    /// position at all and was reaching an uncaught top level as `file: lua error: x` — the file
    /// it had declined, in front of a message that was already carrying one.
    pub fn in_chunk(mut self, chunk: impl Into<String>) -> Self {
        if self.bare {
            return self;
        }
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
    /// module has already unwound past it by the time `pcall` renders this, so the caller is
    /// back on the outer chunk — and naming *that* file with the module's line number is worse
    /// than naming neither.
    pub fn value_string(&self, current: &str) -> String {
        let chunk = self.chunk.as_deref().unwrap_or(current);
        match self.line {
            Some(line) => format!("{chunk}:{line}: {}", self.message),
            None => self.message.clone(),
        }
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
