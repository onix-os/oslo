//! `oslo.db` — a database a config or a plugin owns.
//!
//! ```lua
//! local db = oslo.db.open("notes")
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
//! # A handle is a table of closures
//!
//! `open` answers a table whose functions have captured the open store. That is what makes
//! `db:get(k)` work — Lua's `:` passes the table as the first argument, which these ignore, because
//! the store they act on was decided when the handle was made rather than by what it is called on.
//! The consequence worth knowing: `db.get("k")` with a dot is a mistake this cannot detect, and it
//! reads the key `"k"` from the right database anyway.
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
use std::rc::Rc;

thread_local! {
    /// Databases opened this session, by name.
    ///
    /// **Reopened handles share one store**, because two opens of one file are two writers of one
    /// file, and the second would fail or block depending on how far the engine got. A config that
    /// calls `oslo.db.open("notes")` in two places means one database either way.
    static OPEN: RefCell<HashMap<String, Rc<Store>>> = RefCell::new(HashMap::new());

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
            Err(message) => Ok(vec![Value::Nil, Value::str(message)]),
        }
    });

    // oslo.db.path(name) -> where that database would live, without opening it
    put(&mut db, "path", |_, args| {
        let name = text(&args, 1, "oslo.db.path")?;
        match store::path_of(&name) {
            Some(path) => ok(Value::str(path.to_string_lossy())),
            None => Ok(vec![
                Value::Nil,
                Value::str(format!("{name:?}: not a name")),
            ]),
        }
    });

    Value::table(db)
}

/// The store for `name`, opening it the first time and sharing it after.
fn opened(name: &str) -> Result<Rc<Store>, String> {
    OPEN.with(|open| {
        if let Some(store) = open.borrow().get(name) {
            return Ok(Rc::clone(store));
        }
        let store = Rc::new(store::open(name)?);
        open.borrow_mut()
            .insert(name.to_string(), Rc::clone(&store));
        Ok(store)
    })
}

/// The table `open` answers with.
fn handle(name: &str, store: Rc<Store>) -> Value {
    let mut table = Table::new();

    let it = Rc::clone(&store);
    put(&mut table, "get", move |_, args| {
        let key = text(&args, 2, "db:get")?;
        Ok(vec![match store::get(&it, &key) {
            Some(value) => Value::str(String::from_utf8_lossy(&value)),
            None => Value::Nil,
        }])
    });

    let it = Rc::clone(&store);
    put(&mut table, "has", move |_, args| {
        let key = text(&args, 2, "db:has")?;
        ok(Value::Bool(store::has(&it, &key)))
    });

    let it = Rc::clone(&store);
    put(&mut table, "set", move |_, args| {
        let key = text(&args, 2, "db:set")?;
        let value = text(&args, 3, "db:set")?;
        match store::set(&it, &key, value.as_bytes()) {
            Ok(()) => ok(Value::Bool(true)),
            Err(message) => Ok(vec![Value::Nil, Value::str(message)]),
        }
    });

    let it = Rc::clone(&store);
    put(&mut table, "delete", move |_, args| {
        let key = text(&args, 2, "db:delete")?;
        ok(Value::Bool(store::delete(&it, &key)))
    });

    let it = Rc::clone(&store);
    put(&mut table, "keys", move |_, args| {
        // No prefix means every key, which is what a bare `db:keys()` reads as.
        let prefix = match args.get(1) {
            None | Some(Value::Nil) => String::new(),
            _ => text(&args, 2, "db:keys")?,
        };
        ok(super::util::list(
            store::keys(&it, &prefix).into_iter().map(Value::str),
        ))
    });

    // `write` is a Lua function, not a native one: it has to *call* the caller's callback, and a
    // native cannot re-enter a stackless VM. See [`WRITE`].
    let it = Rc::clone(&store);
    put(&mut table, "__begin", move |_, _| {
        ok(staging_table(&Rc::new(RefCell::new(Vec::new())), &it))
    });
    if let Some(write) = WRITE.with(|slot| slot.borrow().clone()) {
        table.set_str("write", write);
    }

    let it = Rc::clone(&store);
    put(&mut table, "path", move |_, _| {
        ok(Value::str(it.path().to_string_lossy()))
    });

    let it = Rc::clone(&store);
    put(&mut table, "size", move |_, _| {
        ok(Value::int(it.size() as i64))
    });

    table.set_str("name", Value::str(name));
    Value::table(table)
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
            Value::str(format!(
                "{}: the write did not commit",
                store.path().display()
            )),
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
    let mut table = Table::new();

    // The other half of `write`: the shim calls this once the callback has returned, so everything
    // the callback staged lands in one transaction or, if it raised, in none.
    let into = Rc::clone(staged);
    let it = Rc::clone(store);
    put(&mut table, "__commit", move |_, _| commit(&it, &into));

    let into = Rc::clone(staged);
    put(&mut table, "set", move |_, args| {
        let key = text(&args, 2, "w:set")?;
        let value = text(&args, 3, "w:set")?;
        if key.is_empty() || key.len() > store::MAX_KEY || value.len() > store::MAX_VALUE {
            return Err(LuaError::new(format!(
                "w:set: {key:?} or its value is out of range"
            )));
        }
        into.borrow_mut().push(Change::Set(key, value.into_bytes()));
        ok(Value::Bool(true))
    });

    let into = Rc::clone(staged);
    put(&mut table, "delete", move |_, args| {
        let key = text(&args, 2, "w:delete")?;
        into.borrow_mut().push(Change::Delete(key));
        ok(Value::Bool(true))
    });

    Value::table(table)
}

#[cfg(test)]
#[path = "db/tests.rs"]
mod tests;
