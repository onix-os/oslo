//! `oslo.spawn` — work that happens off the prompt, and a callback when it is done.
//!
//! ```lua
//! oslo.spawn{ "git", "status", "--porcelain",
//!   on_exit = function(out, status) oslo.state.set("git.dirty", out ~= "") end }
//! ```
//!
//! # The cost this removes
//!
//! Before this, anything a config wanted to *know* had to be fetched on the spot, blocking whatever
//! asked. The `nix` prompt segment shells out on every draw — 6 ms, measured — because there was no
//! other way to have an answer ready. With this it can ask once, put the answer in `oslo.state`, and
//! draw from that.
//!
//! The machinery already existed and was walled in: [`super::external`] spawns a thread, waits with a
//! deadline and delivers through a channel, for *prompt commands only*.
//!
//! # Where a callback runs
//!
//! **At the same safe point timers use** — never the instant the process exits. The process
//! finishes on its own thread; what it produced waits in a queue until the shell holds nothing and
//! can call Lua. This is not an async runtime: it is one process and one callback.
//!
//! That safe point now includes **an idle prompt**, not only a command boundary. The worker queues
//! its result and nudges [`oslo_base::background`]; the editor's wait returns and delivers it. Before
//! that, a callback for something that finished while you were reading the screen waited for your
//! next keystroke — which for the last command of a session is for ever.
//!
//! A Lua value cannot cross threads — it is `Rc` — so the callback never leaves this one. The worker
//! sends bytes and a status; the handler is looked up here, by id.

use super::util::{ok, put};
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// What a finished process produced. Crosses the thread boundary, so nothing Lua here.
struct Finished {
    id: u64,
    out: String,
    status: i32,
}

thread_local! {
    /// Callbacks waiting for their process, by id. Never leaves the shell's thread.
    static WAITING: RefCell<HashMap<u64, Value>> = RefCell::new(HashMap::new());
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

/// What workers have finished, waiting to be handed back.
fn done() -> &'static Mutex<Vec<Finished>> {
    static DONE: OnceLock<Mutex<Vec<Finished>>> = OnceLock::new();
    DONE.get_or_init(|| Mutex::new(Vec::new()))
}

/// Add `oslo.spawn`.
pub fn install(oslo: &mut Table) {
    put(oslo, "spawn", |_, args| start(&args));
}

/// Read the call and get the process going.
fn start(args: &[Value]) -> LuaResult<Vec<Value>> {
    let Some(Value::Table(spec)) = args.first() else {
        return Err(LuaError::new(
            "oslo.spawn: expects a table — the command and its arguments, plus `on_exit`"
                .to_string(),
        ));
    };
    let spec = spec.borrow();

    let mut argv: Vec<String> = Vec::new();
    for value in spec.sequence() {
        match value {
            Value::Str(word) => argv.push(word.to_string()),
            Value::Number(n) => argv.push(n.to_string()),
            other => {
                return Err(LuaError::new(format!(
                    "oslo.spawn: an argument is a {}, which is not a word",
                    other.type_name()
                )));
            }
        }
    }
    let Some(program) = argv.first().cloned() else {
        return Err(LuaError::new("oslo.spawn: no command to run".to_string()));
    };
    let on_exit = match spec.get(&Value::str("on_exit")) {
        Value::Nil => None,
        it @ Value::Function(_) => Some(it),
        _ => {
            return Err(LuaError::new(
                "oslo.spawn: `on_exit` must be a function".to_string(),
            ));
        }
    };
    // A ceiling rather than a limit anybody should meet — but a background process nobody is
    // waiting for is exactly the kind that is never noticed hanging.
    let timeout = match spec.get(&Value::str("timeout")).as_number() {
        Some(n) if n.as_float() > 0.0 => Some(Duration::from_secs_f64(n.as_float() / 1000.0)),
        _ => None,
    };

    let id = NEXT_ID.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next += 1;
        id
    });
    if let Some(handler) = on_exit {
        WAITING.with(|slot| slot.borrow_mut().insert(id, handler));
    }

    std::thread::spawn(move || {
        let (out, status) = run(&program, &argv[1..], timeout);
        if let Ok(mut done) = done().lock() {
            done.push(Finished { id, out, status });
        }
        // **After the result is queued, never before.** The wake is what makes an idle editor come
        // and look; waking first would have it look at a queue this thread has not filled yet, find
        // nothing, and go back to sleep until the next keystroke — which is the delay this exists
        // to remove. Nothing of the result crosses the pipe: it says only *go and look*.
        oslo_base::background::nudge();
    });

    ok(handle(id))
}

/// What a spawn answers with: `job:cancel()`, which forgets the callback.
///
/// **It does not kill the process**, and says so by its name. Killing something whose output you no
/// longer want is a different decision from not wanting it, and a shell that reaped a `git fetch`
/// because a prompt segment was dismissed would be surprising in a way nobody could debug.
fn handle(id: u64) -> Value {
    super::util::record(vec![(
        "cancel",
        super::util::native("spawn handle", move |_, _| {
            let had = WAITING.with(|slot| slot.borrow_mut().remove(&id).is_some());
            ok(Value::Bool(had))
        }),
    )])
}

/// Run the process. On a worker thread, so blocking here costs nothing.
fn run(program: &str, args: &[String], timeout: Option<Duration>) -> (String, i32) {
    use std::process::{Command, Stdio};
    // Through the shell's own hash table, like `external::run`: `execvp` tries every `$PATH` entry
    // in turn and pays for each miss.
    let resolved = oslo_shell::env::builtins::hash_lookup(program);
    let program: &std::ffi::OsStr = match &resolved {
        Some(path) => path.as_os_str(),
        None => std::ffi::OsStr::new(program),
    };
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Left alone on purpose: a background job's complaints belong on the terminal, where they
        // can be seen, rather than folded into a value a callback may not even read.
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        // 127 is what a shell answers for a command it could not run, so a callback reading the
        // status sees the number it already knows.
        Err(_) => return (String::new(), 127),
    };

    let Some(timeout) = timeout else {
        // `wait_with_output` reads the pipe *and* waits, in the right order — the deadlock this
        // avoids is the one `nix_shell::json` has a test for.
        return match child.wait_with_output() {
            Ok(out) => (
                String::from_utf8_lossy(&out.stdout).into_owned(),
                out.status.code().unwrap_or(-1),
            ),
            Err(_) => (String::new(), 127),
        };
    };

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // 124 is what `timeout(1)` answers, which is the convention a script would expect.
                return (String::new(), 124);
            }
            // `ECHILD`: something reaped it first, which a shell with `SIGCHLD` ignored does
            // routinely. It means finished, not failed — the same case that bit `external::run`.
            Err(_) => break,
        }
    }
    let mut out = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        use std::io::Read;
        let _ = pipe.read_to_string(&mut out);
    }
    (out, 0)
}

/// Hand back whatever finished, calling the callbacks that were waiting.
///
/// Called by the read loop where nothing is held. **The queue is emptied before anything runs**: a
/// callback may spawn again, and appending to a list being drained is how that becomes a deadlock on
/// the mutex it is already inside.
pub fn deliver() {
    let finished: Vec<Finished> = match done().lock() {
        Ok(mut done) => std::mem::take(&mut *done),
        Err(_) => return,
    };
    for job in finished {
        let Some(handler) = WAITING.with(|slot| slot.borrow_mut().remove(&job.id)) else {
            // Cancelled while it ran, or it never had a callback. Both are ordinary.
            continue;
        };
        let args = vec![Value::str(&job.out), Value::int(job.status as i64)];
        if let Err(problem) = crate::lua::engine::call_here(&handler, args) {
            eprintln!("oslo: spawn: {problem}");
        }
    }
}

/// Deliver, but only pay for the lock when something is waiting.
///
/// The shape every caller wants: a session with nothing spawned — every session, nearly always —
/// costs one uncontended `try_lock` rather than a drain.
/// Answers whether a callback ran, so the caller can decide whether a drawn prompt is still true.
pub fn deliver_if_any() -> bool {
    if any_done() {
        deliver();
        return true;
    }
    false
}

/// Whether anything has finished, so the loop can skip the lock.
pub fn any_done() -> bool {
    done().lock().map(|done| !done.is_empty()).unwrap_or(false)
}

#[cfg(test)]
#[path = "spawn/tests.rs"]
mod tests;
