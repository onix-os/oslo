//! `oslo.live` — the small surface another program may ask a running shell about.
//!
//! ```lua
//! oslo.live.serve()            -- bind the socket; nothing listens until this runs
//! oslo.live.path()             -- where it is
//! oslo.live.verbs()            -- what a peer may call
//! oslo.live.stop()             -- unbind
//! ```
//!
//! # Nothing serves by default, and that is the design
//!
//! A shell that binds a socket at startup pays a file, a descriptor and an attack surface in every
//! terminal you open, for a question almost none of them will ever be asked. So a shell serves only
//! when its config says so or a key is pressed:
//!
//! ```lua
//! oslo.keys["ctrl-esc"] = function() oslo.live.serve() end
//! ```
//!
//! The client half costs nothing either way: a shell can *ask* hexe things without serving anything
//! itself, which is the common direction and the cheap one.
//!
//! # The surface is a subset, chosen rather than mirrored
//!
//! `VERBS` is the whole of it, and it is short on purpose. Two rules picked the entries:
//!
//! * **it answers something the asker cannot answer itself.** A shell's live environment is exact
//!   where reading `/proc/<pid>/environ` is a snapshot that has already dropped what a directory
//!   environment changed. That is the case this exists for.
//! * **it does not run anything.** There is no `run`, no `eval`, no `source`. A socket that
//!   executes what a caller sends is remote code execution on somebody's session, and every later
//!   decision would be made in that shadow. Adding one is a separate argument, not a later commit.
//!
//! # Every verb answers from the environment, and none needs Lua
//!
//! That is what lets the server live on a thread — see `live/server.rs`. A verb that needed the VM would
//! turn the server into a queue drained by the read loop, so the constraint is worth keeping.

use oslo_base::value::{Table, Value};
use oslo_shell::env::Environment;
use serde_json::Value as Json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod server;

/// The client library, handed out by `oslo lua-api`.
///
/// Compiled in rather than installed beside the binary: a sibling asks the oslo it is actually
/// talking to, so there is no way to load a stub of one version and speak to another.
pub const CLIENT: &str = include_str!("client.lua");

/// Everything a connected peer may call, and what each answers.
///
/// **The single list.** `dispatch` matches on it, `oslo lua-api --verbs` prints it and the client
/// library names the same strings, so a verb cannot be callable and undocumented or listed and
/// unreachable.
pub const VERBS: &[(&str, &str)] = &[
    ("cwd", "the directory the shell is in"),
    ("session", "this shell's session id"),
    ("verbs", "this list"),
    ("env.get", "one variable, exactly as the shell has it"),
    ("env.all", "every exported variable, as a record"),
    ("env.set", "set one variable in the running shell"),
    ("macros.get", "the body of one stored macro"),
    ("notify", "put a line in the shell's message log"),
];

/// Build the `oslo.live` table.
pub fn build(env: &Arc<Mutex<Environment>>) -> Value {
    let mut it = Table::new();

    let serving = Arc::clone(env);
    super::util::put(&mut it, "serve", move |_, _| {
        match server::start(&serving) {
            Ok(path) => super::util::ok(Value::str(path.to_string_lossy())),
            Err(why) => super::util::failed("oslo.live.serve", why),
        }
    });

    super::util::put(&mut it, "stop", |_, _| {
        let stopped = server::stop();
        // A shell that is not serving should not go on holding a copy of its environment.
        forget();
        super::util::ok(Value::Bool(stopped))
    });

    // Where it is *while serving*, and where it would be otherwise — one function, because a
    // caller printing a diagnostic wants an answer either way.
    super::util::put(&mut it, "path", |_, _| {
        let path = server::serving().unwrap_or_else(|| oslo_base::wire::socket_path("oslo", None));
        super::util::ok(Value::str(path.to_string_lossy()))
    });

    super::util::put(&mut it, "serving", |_, _| {
        super::util::ok(Value::Bool(server::serving().is_some()))
    });

    super::util::put(&mut it, "verbs", |_, _| {
        super::util::ok(super::util::list(VERBS.iter().map(|(name, about)| {
            super::util::record(vec![
                ("name", Value::str(name)),
                ("about", Value::str(about)),
            ])
        })))
    });

    // The client library, for a `.env.lua` or a plugin that wants to talk to another oslo without
    // shelling out to `oslo lua-api` for its own source.
    super::util::put(&mut it, "client", |_, _| {
        super::util::ok(Value::str(CLIENT))
    });

    Value::table(it)
}

/// The exported environment as of the last prompt, for answering while the shell is busy.
///
/// **Why a copy exists at all.** The read loop holds the environment lock for as long as a
/// foreground command runs, so a peer asking "what is your `$PATH`" during a build would be told
/// the shell is busy — which is exactly the moment somebody asks. A snapshot taken at each prompt
/// costs one clone of a small map, and only in a shell that is actually serving.
///
/// It is a *fallback*, never the first answer: the live environment is tried first and this used
/// only when the lock is held (see `dispatch`). So an idle shell is exact and a busy one is one command stale, which
/// is still better than `/proc/<pid>/environ` — that is stale since `exec`.
static SNAPSHOT: std::sync::RwLock<Option<Vec<(String, String)>>> = std::sync::RwLock::new(None);

/// Take the snapshot. Called from the read loop before each prompt.
///
/// Does nothing at all unless a socket is bound, so a shell that never serves never pays for this.
pub fn publish(env: &Arc<Mutex<Environment>>) {
    let Some(path) = server::serving() else {
        return;
    };
    // `try_lock`: this runs on the read loop, which is about to want the lock itself. Missing one
    // prompt's snapshot is nothing — the next one is a keystroke away.
    let Ok(mut held) = env.try_lock() else {
        return;
    };

    // **`$OSLO_SOCK`, so a child finds the shell that started it.** Without it a program asking
    // "which oslo?" falls back to the newest socket in the directory, which is a guess — and once
    // every shell serves, it is usually the wrong one. A child inherits this through `execve` and
    // needs no discovery at all.
    //
    // Set here rather than in `serve()` because the two places that call `serve()` — a config line
    // and a key handler — are both already holding this lock, and a `serve()` that waited for it
    // would deadlock the shell that asked.
    let sock = path.to_string_lossy();
    if held.get_var("OSLO_SOCK") != Some(&*sock) {
        held.set_var("OSLO_SOCK", &sock, true);
    }

    let taken = held.exported_vars();
    drop(held);
    if let Ok(mut slot) = SNAPSHOT.write() {
        *slot = Some(taken);
    }
}

/// Unbind and forget, for a shell that is ending. See `startup::repl::session::fire_exit`.
pub fn stop_serving() {
    server::stop();
    forget();
}

/// Forget it, so a shell that stops serving stops holding a copy of its environment.
pub fn forget() {
    if let Ok(mut slot) = SNAPSHOT.write() {
        *slot = None;
    }
}

/// One variable from the snapshot.
fn remembered(name: &str) -> Option<Option<String>> {
    let slot = SNAPSHOT.read().ok()?;
    let vars = slot.as_ref()?;
    Some(
        vars.iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone()),
    )
}

/// A reply frame. `ok` carries the call's return values; `error` carries why not.
pub(crate) struct Reply;

impl Reply {
    pub(crate) fn ok(values: Vec<Json>) -> String {
        serde_json::json!({ "ok": true, "n": values.len(), "result": values }).to_string()
    }

    pub(crate) fn failed(why: &str) -> String {
        serde_json::json!({ "ok": false, "error": why }).to_string()
    }
}

/// Run one verb.
///
/// **An unknown name is refused by name.** A server that answered `nil` for a call it does not have
/// would be indistinguishable from one whose verb returned nothing, and a client written against a
/// newer oslo would fail silently against an older one.
pub(crate) fn dispatch(
    call: &str,
    args: &[Json],
    env: &Arc<Mutex<Environment>>,
    wait: Duration,
) -> Result<Vec<Json>, String> {
    let text = |n: usize| -> Result<String, String> {
        args.get(n)
            .and_then(Json::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("{call}: argument {} must be a string", n + 1))
    };

    match call {
        "verbs" => Ok(vec![Json::Array(
            VERBS
                .iter()
                .map(|(name, about)| serde_json::json!({ "name": name, "about": about }))
                .collect(),
        )]),
        "session" => Ok(vec![Json::String(oslo_base::track::session::id())]),
        "cwd" => Ok(vec![Json::String(
            std::env::current_dir()
                .map_err(|e| format!("cwd: {e}"))?
                .to_string_lossy()
                .into_owned(),
        )]),
        // The two reads fall back to the snapshot, so a shell running a build still answers. See
        // [`SNAPSHOT`] for why that is a fallback and not the primary path.
        "env.get" => {
            let name = text(0)?;
            let value = match hold(env, wait) {
                Ok(held) => held.get_var(&name).map(str::to_string),
                Err(busy) => remembered(&name).ok_or(busy)?,
            };
            Ok(vec![match value {
                Some(value) => Json::String(value),
                None => Json::Null,
            }])
        }
        "env.all" => {
            let vars = match hold(env, wait) {
                Ok(held) => held.exported_vars(),
                Err(busy) => SNAPSHOT
                    .read()
                    .ok()
                    .and_then(|slot| slot.clone())
                    .ok_or(busy)?,
            };
            let all = vars
                .into_iter()
                .map(|(name, value)| (name, Json::String(value)))
                .collect::<serde_json::Map<_, _>>();
            Ok(vec![Json::Object(all)])
        }
        "env.set" => {
            let (name, value) = (text(0)?, text(1)?);
            let mut held = hold(env, wait)?;
            // Exported, because a variable a peer sets is one it wants a child to see — that is the
            // whole reason for setting it from outside.
            Ok(vec![Json::Bool(held.set_var(&name, &value, true))])
        }
        "macros.get" => {
            let name = text(0)?;
            let found = oslo_base::macros::live::want()
                .into_iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.body);
            Ok(vec![match found {
                Some(body) => Json::String(body),
                None => Json::Null,
            }])
        }
        "notify" => {
            let line = text(0)?;
            oslo_base::messages::say(oslo_base::messages::Level::Note, "live", &line);
            Ok(vec![Json::Bool(true)])
        }
        other => Err(format!(
            "{other}: not a verb this shell answers — try `oslo lua-api --verbs`"
        )),
    }
}

/// Take the environment lock, or give up and say so.
///
/// **Never a blocking `lock`.** A foreground command holds this for as long as it runs, and a server
/// thread parked on it would answer long after the client's own deadline had passed. Saying the
/// shell is busy is a true answer that arrives in time to be useful.
fn hold<'a>(
    env: &'a Arc<Mutex<Environment>>,
    wait: Duration,
) -> Result<std::sync::MutexGuard<'a, Environment>, String> {
    let deadline = Instant::now() + wait;
    loop {
        match env.try_lock() {
            Ok(held) => return Ok(held),
            Err(std::sync::TryLockError::Poisoned(held)) => return Ok(held.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => return Err("the shell is busy running something".to_string()),
        }
    }
}

#[cfg(test)]
#[path = "live/tests.rs"]
mod tests;
