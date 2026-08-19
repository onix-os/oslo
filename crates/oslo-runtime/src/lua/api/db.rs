//! `oslo.db` — a database a config or a plugin owns.
//!
//! ```lua
//! local db <close> = oslo.db.open("notes")
//! db:set("last", os.date())
//! print(db:get("last"))
//! for _, key in ipairs(db:keys("draft/")) do print(key) end
//! db:write(function(w)          -- one transaction; nothing lands if it raises
//!   w:set("a", "1")
//!   w:delete("b")
//! end)
//! ```
//!
//! **In every build, not behind the plugin feature.** A database is a capability, not a policy: a
//! config that will never install a plugin can still want somewhere durable to put something, and a
//! surface that exists only in some builds means every caller has to ask whether it is there.
//!
//! # A handle is an object
//!
//! `open` answers a table with a metatable: the verbs sit behind `__index`, so `pairs(db)` shows
//! nothing, a typo (`db.nmae = 1`) is refused, and `<close>` releases the store at the end of the
//! block. The verbs still ignore `self` — the store they act on was decided when the handle was
//! made — but they read their first real argument at position 2, so `db.get("k")` with a dot is a
//! message rather than a silent read of the wrong key. See [`super::handle`].
//!
//! Closing shuts the file: the store is held in one place all the verbs share, `__close` empties it,
//! and the session's own map of open databases is weak so that emptying is enough. A handle without
//! `<close>` is released when it is collected — the same weak map is what makes that work, with no
//! `__gc` involved. `<close>` buys the *moment*, which is what matters when a config opens sixty.
//!
//! # Values are bytes
//!
//! A Lua string is a byte string, and so is a value here — no encoding, no trimming, no trailing
//! newline. `oslo.json` is how structure gets in and out, which keeps this module from having an
//! opinion about what anybody stores.

use super::util::{ok, put, text};
use oslo_base::store;
use oslo_base::track::kv::Store;
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use oslo_luavm::Host;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

/// The store a handle acts on, until the handle is closed.
///
/// **Shared, and emptiable.** Every verb holds one of these rather than an `Rc<Store>` of its own,
/// so `__close` can put the store down for all of them at once — which is what makes closing a
/// release rather than only a refusal. Once it is empty the file is shut, because nothing else in
/// the session was keeping it open; see [`OPEN`].
type Held = Rc<RefCell<Option<Rc<Store>>>>;

thread_local! {
    /// Databases open in this session, by name.
    ///
    /// **Reopened handles share one store**, because two opens of one file are two writers of one
    /// file, and the second would fail or block depending on how far the engine got. A config that
    /// calls `oslo.db.open("notes")` in two places means one database either way.
    ///
    /// **Weakly, so the cache is not itself an owner.** Sharing is the point; keeping a store alive
    /// after every handle on it has been closed is not, and a strong entry here would make
    /// `<close>` unable to shut anything.
    static OPEN: RefCell<HashMap<String, Weak<Store>>> = RefCell::new(HashMap::new());

    /// `db:write`, compiled once at install time.
    ///
    /// **In Lua, because it calls the caller's function.** Every change in one transaction means
    /// running a callback between opening the batch and applying it — and a Rust native cannot do
    /// that on a stackless VM, which unwinds Lua's stack into its own heap rather than Rust's. The
    /// two ends are natives (`__begin`, `__commit`) and the middle is three lines of Lua, where
    /// calling a function is free.
    ///
    /// An error in the callback propagates and `__commit` never runs, so nothing is written —
    /// which is the guarantee `db:write` exists to give.
    static WRITE: RefCell<Option<Value>> = const { RefCell::new(None) };
}

/// The Lua half of `db:write`. See [`WRITE`].
const WRITE_SHIM: &str = "return function(self, work)
    if type(work) ~= 'function' then
        error('db:write: expected a function of one argument', 2)
    end
    local w = self:__begin()
    work(w)
    return w:__commit()
end";

/// Build the `oslo.db` table.
pub fn build(host: &dyn Host) -> Value {
    // Compiled here rather than per open: one function serves every handle.
    if let Ok(values) = host.eval(WRITE_SHIM, "=oslo.db")
        && let Some(write @ Value::Function(_)) = values.into_iter().next()
    {
        WRITE.with(|slot| *slot.borrow_mut() = Some(write));
    }

    let mut db = Table::new();

    // oslo.db.open(name) -> handle, or nil + message
    put(&mut db, "open", |_, args| {
        let name = text(&args, 1, "oslo.db.open")?;
        match opened(&name) {
            Ok(store) => ok(handle(&name, store)),
            // The name is carried alongside the sentence, so a caller that opens several does not
            // have to read English to find out which one refused.
            Err(message) => Ok(vec![
                Value::Nil,
                super::problem::new(
                    message,
                    vec![("name", Value::str(&name)), ("kind", Value::str("open"))],
                ),
            ]),
        }
    });

    // oslo.db.path(name) -> where that database would live, without opening it
    put(&mut db, "path", |_, args| {
        let name = text(&args, 1, "oslo.db.path")?;
        match store::path_of(&name) {
            Some(path) => ok(Value::str(path.to_string_lossy())),
            None => Ok(vec![
                Value::Nil,
                super::problem::new(
                    format!("{name:?}: not a name"),
                    vec![("name", Value::str(&name)), ("kind", Value::str("name"))],
                ),
            ]),
        }
    });

    Value::table(db)
}

/// The store for `name`, opening it the first time and sharing it after.
fn opened(name: &str) -> Result<Rc<Store>, String> {
    OPEN.with(|open| {
        if let Some(store) = open.borrow().get(name).and_then(Weak::upgrade) {
            return Ok(store);
        }
        let store = Rc::new(store::open(name)?);
        open.borrow_mut()
            .insert(name.to_string(), Rc::downgrade(&store));
        Ok(store)
    })
}

/// The handle `open` answers with — an object, not a table of closures.
///
/// The verbs live behind `__index`, so `__begin` and the rest of `db:write`'s machinery are no
/// longer part of the surface, and `pairs(db)` has nothing in it to get wrong. See
/// [`super::handle`].
fn handle(name: &str, store: Rc<Store>) -> Value {
    let mut table = super::handle::Handle::new("oslo.db");
    let held: Held = Rc::new(RefCell::new(Some(store)));

    let it = Rc::clone(&held);
    table.verb("get", move |_, args| {
        let store = open(&it, "db:get")?;
        let key = text(&args, 2, "db:get")?;
        Ok(vec![match store::get(&store, &key) {
            // The bytes as they were stored. This module has always said a value is bytes; until
            // the shell had a value that could hold them, it was not quite true.
            Some(value) => Value::bytes(&value),
            None => Value::Nil,
        }])
    });

    let it = Rc::clone(&held);
    table.verb("has", move |_, args| {
        let store = open(&it, "db:has")?;
        let key = text(&args, 2, "db:has")?;
        ok(Value::Bool(store::has(&store, &key)))
    });

    let it = Rc::clone(&held);
    table.verb("set", move |_, args| {
        let store = open(&it, "db:set")?;
        let key = text(&args, 2, "db:set")?;
        let value = super::util::raw(&args, 3, "db:set")?;
        match store::set(&store, &key, &value) {
            Ok(()) => ok(Value::Bool(true)),
            Err(message) => Ok(vec![
                Value::Nil,
                super::problem::new(
                    message,
                    vec![("key", Value::str(&key)), ("kind", Value::str("write"))],
                ),
            ]),
        }
    });

    let it = Rc::clone(&held);
    table.verb("delete", move |_, args| {
        let store = open(&it, "db:delete")?;
        let key = text(&args, 2, "db:delete")?;
        ok(Value::Bool(store::delete(&store, &key)))
    });

    let it = Rc::clone(&held);
    table.verb("keys", move |_, args| {
        let store = open(&it, "db:keys")?;
        // No prefix means every key, which is what a bare `db:keys()` reads as.
        let prefix = match args.get(1) {
            None | Some(Value::Nil) => String::new(),
            _ => text(&args, 2, "db:keys")?,
        };
        ok(super::util::list(
            store::keys(&store, &prefix).into_iter().map(Value::str),
        ))
    });

    // `write` is a Lua function, not a native one: it has to *call* the caller's callback, and a
    // native cannot re-enter a stackless VM. See [`WRITE`]. `__begin` is a verb like any other and
    // so lives behind `__index`, which is what keeps it out of `pairs(db)`.
    let it = Rc::clone(&held);
    table.verb("__begin", move |_, _| {
        let store = open(&it, "db:write")?;
        ok(staging_table(&Rc::new(RefCell::new(Vec::new())), &store))
    });
    if let Some(write) = WRITE.with(|slot| slot.borrow().clone()) {
        table.field("write", write);
    }

    let it = Rc::clone(&held);
    table.verb("path", move |_, _| {
        ok(Value::str(open(&it, "db:path")?.path().to_string_lossy()))
    });

    let it = Rc::clone(&held);
    table.verb("size", move |_, _| {
        ok(Value::int(open(&it, "db:size")?.size() as i64))
    });

    table.field("name", Value::str(name));

    // **Closing shuts the file**, rather than only refusing the verbs: every verb reaches the store
    // through `held`, so emptying it drops the last reference this handle had, and [`OPEN`] holds
    // only a `Weak`. A second handle on the same name keeps its own, so this cannot shut a database
    // somebody else is still using.
    let it = Rc::clone(&held);
    table.on_close("oslo.db.close", move || {
        it.borrow_mut().take();
    });

    table.build()
}

/// The store behind a handle, or the message for one that has been closed.
///
/// [`Handle::verb`](super::handle::Handle::verb) refuses a closed handle before a verb runs, so
/// reaching this is the case where a store was released some other way — which is a message rather
/// than a panic, because a config should never be able to abort the shell.
fn open(held: &Held, verb: &str) -> LuaResult<Rc<Store>> {
    held.borrow()
        .clone()
        .ok_or_else(|| LuaError::new(format!("{verb}: the database is closed")))
}

/// Apply everything staged on `writer`, as one transaction.
fn commit(store: &Rc<Store>, staged: &Rc<RefCell<Vec<Change>>>) -> LuaResult<Vec<Value>> {
    let changes = staged.borrow();
    let applied = store.write(|writer| {
        for change in changes.iter() {
            match change {
                Change::Set(key, value) => {
                    writer.put(
                        oslo_base::track::kv::Tree::Plugin,
                        key.as_bytes().to_vec(),
                        value.clone(),
                    )?;
                }
                Change::Delete(key) => {
                    writer.delete(oslo_base::track::kv::Tree::Plugin, key.as_bytes());
                }
            }
        }
        Some(())
    });
    match applied {
        Some(()) => ok(Value::Bool(true)),
        None => Ok(vec![
            Value::Nil,
            super::problem::new(
                format!("{}: the write did not commit", store.path().display()),
                vec![
                    ("path", Value::str(store.path().to_string_lossy())),
                    ("kind", Value::str("write")),
                ],
            ),
        ]),
    }
}

/// One staged change.
enum Change {
    Set(String, Vec<u8>),
    Delete(String),
}

/// The `w` a `db:write` callback is handed: the same two verbs, recording rather than writing.
fn staging_table(staged: &Rc<RefCell<Vec<Change>>>, store: &Rc<Store>) -> Value {
    let mut table = super::handle::Handle::new("oslo.db.writer");

    // The other half of `write`: the shim calls this once the callback has returned, so everything
    // the callback staged lands in one transaction or, if it raised, in none.
    let into = Rc::clone(staged);
    let it = Rc::clone(store);
    table.verb("__commit", move |_, _| commit(&it, &into));

    let into = Rc::clone(staged);
    table.verb("set", move |_, args| {
        let key = text(&args, 2, "w:set")?;
        let value = super::util::raw(&args, 3, "w:set")?;
        if key.is_empty() || key.len() > store::MAX_KEY || value.len() > store::MAX_VALUE {
            return Err(LuaError::new(format!(
                "w:set: {key:?} or its value is out of range"
            )));
        }
        into.borrow_mut().push(Change::Set(key, value));
        ok(Value::Bool(true))
    });

    let into = Rc::clone(staged);
    table.verb("delete", move |_, args| {
        let key = text(&args, 2, "w:delete")?;
        into.borrow_mut().push(Change::Delete(key));
        ok(Value::Bool(true))
    });

    table.build()
}

#[cfg(test)]
#[path = "db/tests.rs"]
mod tests;
