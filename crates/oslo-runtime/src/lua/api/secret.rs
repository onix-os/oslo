//! `oslo.secret` — the secrets store, from Lua.
//!
//! ```lua
//! local value = oslo.secret.get("gh-token")        -- the store this shell would use
//! oslo.secret.set("gh-token", value)
//! for _, name in ipairs(oslo.secret.list()) do print(name) end
//!
//! local work = oslo.secret.open("work")            -- another store by name
//! print(work:get("deploy"))
//!
//! local mine = oslo.secret.mine()                  -- a plugin's own encrypted store
//! mine:set("cursor", "42")
//!
//! local sealed = oslo.secret.seal("anything")      -- base64 of an age file
//! print(oslo.secret.unseal(sealed))
//! ```
//!
//! # Three doors, on purpose
//!
//! `get`/`set`/`list` act on the store this invocation would use — `$OSLO_SECRET_STORE`, the
//! `default` line, or `user`. `open(name)` names another. `mine()` is a plugin's own, and takes no
//! name because the name is not the plugin's to write.
//!
//! # What a plugin may reach
//!
//! **`plugin.` is refused from Lua, always, to every caller.** The check is on the *name asked for*
//! rather than on who is asking, so it cannot be sidestepped by deferring the call into a hook or a
//! timer — and `mine()` stays the only door to a plugin store. The command line has no such rule:
//! `oslo secret --store plugin.notes list` works, because you must be able to see and remove what a
//! plugin kept on your machine.
//!
//! **A handle acquired while a plugin is loading is filtered to the names its manifest declared.**
//! That is a disclosure and not a sandbox, and the difference matters: the declaration is shown by
//! `oslo plugin install` before trust is decided, but a plugin can call `oslo secret get` as a
//! subprocess and nothing here stops it. What it buys is a plugin that can be caught contradicting
//! its own manifest — see `docs/features/secrets.md`.
//!
//! # Bytes, and base64
//!
//! A value is a Lua string, which is a byte string. `seal` and `unseal` are the exception: they
//! carry a whole age *file*, which is binary, so they speak base64 rather than putting a NUL and a
//! header through anything that might treat it as text.

mod defined;

use super::util::{ok, put, text};
use oslo_base::secrets::{self, Store};
use oslo_lua::value::{Table, Value};

/// Build the `oslo.secret` table.
pub fn build() -> Value {
    let mut secret = Table::new();

    // oslo.secret.open(name) -> handle, or nil + message
    put(&mut secret, "open", |_, args| {
        let name = text(&args, 1, "oslo.secret.open")?;
        match reachable(&name).and_then(|()| Store::named(&name)) {
            // A store Lua defined keeps its own bytes; everything else is a directory of files.
            Ok(store) if defined::holds(&name) => ok(defined::handle(store)),
            Ok(store) => ok(handle(store, allowed())),
            Err(message) => Ok(vec![Value::Nil, Value::str(message)]),
        }
    });

    // oslo.secret.define(name, handlers) -> where this store's bytes live
    put(&mut secret, "define", |_, args| defined::define(&args));

    // oslo.secret.mine() -> the calling plugin's own store
    #[cfg(feature = "plugin")]
    put(&mut secret, "mine", |_, _| {
        let Some(who) = who() else {
            return Ok(vec![
                Value::Nil,
                Value::str("oslo.secret.mine: only a plugin has one, and only while it loads"),
            ]);
        };
        match Store::named(&format!("{}{}", secrets::PLUGIN, who.plugin)) {
            // Its own store, and no filter on it: the names in there are the plugin's own.
            Ok(store) => ok(handle(store, None)),
            Err(message) => Ok(vec![Value::Nil, Value::str(message)]),
        }
    });

    // oslo.secret.stores() -> every store this machine declares
    put(&mut secret, "stores", |_, _| {
        let declared = match secrets::conf::read() {
            Ok(conf) => conf,
            Err(message) => return Ok(vec![Value::Nil, Value::str(message)]),
        };
        let mut names: Vec<String> = declared
            .sections
            .iter()
            .map(|section| section.name.clone())
            .filter(|name| !name.starts_with(secrets::PLUGIN))
            .collect();
        if !names.iter().any(|name| name == secrets::USER) {
            names.insert(0, secrets::USER.to_string());
        }
        for name in defined::names() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
        ok(list(&names))
    });

    for (verb, on_selected) in DIRECT {
        put(&mut secret, verb, move |_, args| {
            let store = match Store::selected(None) {
                Ok(store) => store,
                Err(message) => return Ok(vec![Value::Nil, Value::str(message)]),
            };
            on_selected(&store, &args)
        });
    }

    Value::table(secret)
}

/// What a bare `oslo.secret.X` does, against the store this shell would use.
type Direct = fn(&Store, &[Value]) -> oslo_lua::LuaResult<Vec<Value>>;
const DIRECT: &[(&str, Direct)] = &[
    ("get", |store, args| {
        let name = text(args, 1, "oslo.secret.get")?;
        Ok(read(store, &name, allowed()))
    }),
    ("set", |store, args| {
        let name = text(args, 1, "oslo.secret.set")?;
        let value = text(args, 2, "oslo.secret.set")?;
        Ok(wrote(store, &name, &value, allowed()))
    }),
    ("list", |store, _| ok(list(&store.names()))),
    ("seal", |store, args| {
        let value = text(args, 1, "oslo.secret.seal")?;
        Ok(match store.seal(value.as_bytes()) {
            Ok(sealed) => vec![Value::str(base64(&sealed))],
            Err(message) => vec![Value::Nil, Value::str(message)],
        })
    }),
    ("unseal", |store, args| {
        let sealed = text(args, 1, "oslo.secret.unseal")?;
        let Some(bytes) = unbase64(&sealed) else {
            return Ok(vec![Value::Nil, Value::str("not base64")]);
        };
        Ok(match store.unseal(&bytes) {
            Ok(value) => vec![Value::str(String::from_utf8_lossy(&value))],
            Err(message) => vec![Value::Nil, Value::str(message)],
        })
    }),
];

/// The plugin whose file is running, when there is one and this build has plugins at all.
///
/// A build without the `plugin` feature has no loader, so nothing is ever loading and every caller
/// is the config or the prompt.
#[cfg(feature = "plugin")]
fn who() -> Option<crate::plugin::loading::Loading> {
    crate::plugin::loading::current()
}

/// The names this caller may read, or `None` for all of them.
///
/// Taken at the moment a handle is made or a call is answered, which is what makes it attribution:
/// it is the manifest of whatever plugin is loading, and nothing at all otherwise.
#[cfg(feature = "plugin")]
fn allowed() -> Option<Vec<String>> {
    who().map(|it| it.secrets)
}

#[cfg(not(feature = "plugin"))]
fn allowed() -> Option<Vec<String>> {
    None
}

/// Whether Lua may name this store at all.
fn reachable(name: &str) -> Result<(), String> {
    match name.starts_with(secrets::PLUGIN) {
        true => Err(format!(
            "{name}: a plugin's store is reached with oslo.secret.mine(), not by name"
        )),
        false => Ok(()),
    }
}

/// The table `open` and `mine` answer with.
fn handle(store: Store, allowed: Option<Vec<String>>) -> Value {
    let mut table = Table::new();

    let (it, may) = (store.clone(), allowed.clone());
    put(&mut table, "get", move |_, args| {
        let name = text(&args, 2, "store:get")?;
        Ok(read(&it, &name, may.clone()))
    });

    let (it, may) = (store.clone(), allowed.clone());
    put(&mut table, "set", move |_, args| {
        let name = text(&args, 2, "store:set")?;
        let value = text(&args, 3, "store:set")?;
        Ok(wrote(&it, &name, &value, may.clone()))
    });

    let (it, may) = (store.clone(), allowed.clone());
    put(&mut table, "forget", move |_, args| {
        let name = text(&args, 2, "store:forget")?;
        if let Some(refused) = refusal(&name, may.clone()) {
            return Ok(vec![Value::Nil, Value::str(refused)]);
        }
        Ok(match it.forget(&name) {
            Ok(()) => vec![Value::Bool(true)],
            Err(message) => vec![Value::Nil, Value::str(message)],
        })
    });

    let (it, may) = (store.clone(), allowed);
    put(&mut table, "list", move |_, _| {
        // A filtered handle lists what it could read, not what is there: the names in a store are
        // themselves information, and a plugin allowed one of them has been told about one.
        let names = match &may {
            Some(allowed) => it
                .names()
                .into_iter()
                .filter(|name| allowed.contains(name))
                .collect(),
            None => it.names(),
        };
        ok(list(&names))
    });

    let it = store.clone();
    put(&mut table, "where", move |_, _| {
        ok(Value::str(it.directory.to_string_lossy()))
    });

    let it = store.clone();
    put(&mut table, "seal", move |_, args| {
        let value = text(&args, 2, "store:seal")?;
        Ok(match it.seal(value.as_bytes()) {
            Ok(sealed) => vec![Value::str(base64(&sealed))],
            Err(message) => vec![Value::Nil, Value::str(message)],
        })
    });

    let it = store;
    put(&mut table, "unseal", move |_, args| {
        let sealed = text(&args, 2, "store:unseal")?;
        let Some(bytes) = unbase64(&sealed) else {
            return Ok(vec![Value::Nil, Value::str("not base64")]);
        };
        Ok(match it.unseal(&bytes) {
            Ok(value) => vec![Value::str(String::from_utf8_lossy(&value))],
            Err(message) => vec![Value::Nil, Value::str(message)],
        })
    });

    Value::table(table)
}

fn read(store: &Store, name: &str, may: Option<Vec<String>>) -> Vec<Value> {
    if let Some(refused) = refusal(name, may) {
        return vec![Value::Nil, Value::str(refused)];
    }
    match store.get(name) {
        Ok(value) => vec![Value::str(String::from_utf8_lossy(&value))],
        Err(message) => vec![Value::Nil, Value::str(message)],
    }
}

fn wrote(store: &Store, name: &str, value: &str, may: Option<Vec<String>>) -> Vec<Value> {
    if let Some(refused) = refusal(name, may) {
        return vec![Value::Nil, Value::str(refused)];
    }
    match store.set(name, value.as_bytes()) {
        Ok(()) => vec![Value::Bool(true)],
        Err(message) => vec![Value::Nil, Value::str(message)],
    }
}

/// Why this caller may not touch `name`, if it may not.
fn refusal(name: &str, may: Option<Vec<String>>) -> Option<String> {
    let allowed = may?;
    if allowed.iter().any(|declared| declared == name) {
        return None;
    }
    Some(format!(
        "{name}: not declared in this plugin's `secrets = {{ … }}`"
    ))
}

fn list(names: &[String]) -> Value {
    let mut table = Table::new();
    for (at, name) in names.iter().enumerate() {
        table.set(Value::int(at as i64 + 1), Value::str(name.clone()));
    }
    Value::table(table)
}

/// **Base64 rather than the bytes**, and rather than armour: an age file is binary, and `armor`
/// costs 45 KB measured for a format nothing outside this boundary would read.
pub(crate) fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn unbase64(text: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()
}
