//! A handle the shell hands out: an object with a metatable, not a table of closures.
//!
//! **The difference the VM made.** The evaluator this replaced had no working metatables, so every
//! handle — a database, a spawned process — was a plain table whose keys *were* its verbs. Four
//! things follow from that, and one metatable settles all four:
//!
//! * **Internals are public.** `db:write` is implemented as two natives with a Lua shim between
//!   them, and `__begin` and `__commit` sat in the same table as `get` and `set`, visible to
//!   `pairs` and callable by anyone.
//! * **A typo is a silent new field.** `db.nmae = 1` added a key to the handle. Nothing refused it.
//! * **Nothing is ever released.** There was no `__close`, so a handle held its resources until the
//!   session ended, however short the block that used it.
//! * **A closed handle still works.** With nothing to close, there was nothing to refuse either.
//!
//! With a metatable the verbs live behind `__index`, `pairs` over a handle shows nothing to get
//! wrong, `__newindex` refuses the typo, `<close>` releases at the end of the block, and every verb
//! refuses once that has happened:
//!
//! ```lua
//! local db <close> = oslo.db.open("notes")
//! db:set("last", os.date())
//! ```
//!
//! `__name` is set so an error message calls it `oslo.db` rather than `table: 0x…`.

use super::util::native;
use oslo_base::value::{LuaError, LuaResult, Table, Value};
use oslo_luavm::Host;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A handle under construction.
pub(super) struct Handle {
    /// What `__index` answers from: the verbs and the plain fields together.
    verbs: Table,
    /// What `__name` reports, and what an error message calls this thing.
    name: &'static str,
    /// What `tostring` answers, for a handle that stands for something with a name of its own.
    shown: Option<String>,
    /// What `__close` runs, if anything.
    closer: Option<Value>,
    /// What calling the handle runs, if it is callable at all.
    called: Option<Value>,
    /// Shared with every verb, so closing the handle is something they can all see.
    closed: Rc<Cell<bool>>,
}

impl Handle {
    pub(super) fn new(name: &'static str) -> Self {
        Handle {
            verbs: Table::new(),
            name,
            shown: None,
            closer: None,
            called: None,
            closed: Rc::new(Cell::new(false)),
        }
    }

    /// Add a method, reached as `handle:verb(…)`.
    ///
    /// The wrapper is the use-after-close check. Doing it here rather than in each verb is what
    /// makes "closed" mean the same thing for every handle in the API.
    pub(super) fn verb(
        &mut self,
        name: &'static str,
        f: impl Fn(&dyn Host, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
    ) -> &mut Self {
        let closed = Rc::clone(&self.closed);
        let owner = self.name;
        self.verbs.set_str(
            name,
            native(name, move |host, args| {
                if closed.get() {
                    return Err(LuaError::new(format!(
                        "{owner}:{name}: the handle is closed"
                    )));
                }
                f(host, args)
            }),
        );
        self
    }

    /// Make the handle itself callable, as `handle(…)`.
    ///
    /// **Which is what lets one value be both an iterator and a thing that closes.** A generic
    /// `for` needs something callable; releasing at the end of a block needs something with a
    /// metatable. `__call` is how `for line in oslo.lines{…} do` and
    /// `local out <close> = oslo.lines{…}` are the same handle rather than two returns the caller
    /// has to keep together.
    ///
    /// The call is handed `self` first, exactly as a `:` call is.
    pub(super) fn calls(
        &mut self,
        name: &'static str,
        f: impl Fn(&dyn Host, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
    ) -> &mut Self {
        let closed = Rc::clone(&self.closed);
        let owner = self.name;
        self.called = Some(native(name, move |host, args| {
            if closed.get() {
                return Err(LuaError::new(format!("{owner}: the handle is closed")));
            }
            f(host, args)
        }));
        self
    }

    /// Add a plain value, reached as `handle.field`.
    pub(super) fn field(&mut self, name: &str, value: Value) -> &mut Self {
        self.verbs.set_str(name, value);
        self
    }

    /// What to run when the handle leaves a `<close>` scope.
    ///
    /// Called at most once, and every verb refuses afterwards: a release that runs twice is a bug
    /// the caller cannot see, and so is a verb that acts on something already given up.
    ///
    /// **No handle sets `__gc`, and the reason is different for the two halves of the list.**
    /// Finalizers do run — but for a database, a file or a pipe there is nothing for one to do: the
    /// verbs hold Rust values, and collecting the handle drops them, which shuts the descriptor
    /// without any Lua being involved. `<close>` buys *promptness*, not the release itself. For a
    /// spawn, a timer or a temporary directory a finalizer would be actively wrong: their handles
    /// are normally written for the effect and thrown away, so `__gc` would cancel the callback,
    /// stop the timer and `remove_dir_all` a path somebody had copied out.
    /// It is also the `close` verb, so `h:close()` and leaving a `<close>` scope are the same thing
    /// rather than two things a caller has to know are the same. Registered directly, without the
    /// refusal every other verb gets: closing twice is not a mistake worth an error.
    pub(super) fn on_close(&mut self, name: &'static str, f: impl Fn() + 'static) -> &mut Self {
        let closed = Rc::clone(&self.closed);
        // Answers whether *this* call did the closing, the way `t:stop()` and `job:cancel()` do, so
        // a second `h:close()` is `false` rather than an error. `__close` ignores what it is given.
        let closer = native(name, move |_, _| {
            let first = !closed.replace(true);
            if first {
                f();
            }
            Ok(vec![Value::Bool(first)])
        });
        self.verbs.set_str("close", closer.clone());
        self.closer = Some(closer);
        self
    }

    /// What `tostring` answers, and so what `print` and `..` show.
    ///
    /// Worth setting where the handle *stands for* something a caller would otherwise have to reach
    /// for by name — a temporary directory is its path — so the obvious line prints the useful
    /// thing rather than `oslo.fs.tempdir: 0x…`.
    pub(super) fn shows(&mut self, text: impl Into<String>) -> &mut Self {
        self.shown = Some(text.into());
        self
    }

    /// The handle itself: an empty table wearing the metatable.
    pub(super) fn build(self) -> Value {
        let name = self.name;
        let mut meta = Table::new();
        meta.set_str("__index", Value::table(self.verbs));
        meta.set_str("__name", Value::str(name));
        // A handle is not a place to keep things. Refusing the write turns `db.nmae = 1` — which
        // used to succeed and do nothing — into the mistake it is.
        meta.set_str(
            "__newindex",
            native("__newindex", move |_, args| {
                let field = args
                    .get(1)
                    .map(Value::to_display)
                    .unwrap_or_else(|| "?".to_string());
                Err(LuaError::new(format!(
                    "{name}: cannot set {field:?} on a handle; it is an object, not a table"
                )))
            }),
        );
        if let Some(shown) = self.shown {
            meta.set_str(
                "__tostring",
                native("__tostring", move |_, _| Ok(vec![Value::str(&shown)])),
            );
        }
        if let Some(called) = self.called {
            meta.set_str("__call", called);
        }
        if let Some(closer) = self.closer {
            meta.set_str("__close", closer);
        }

        let mut handle = Table::new();
        handle.metatable = Some(Rc::new(RefCell::new(meta)));
        Value::table(handle)
    }
}
