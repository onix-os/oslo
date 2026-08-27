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
use std::collections::{HashMap, HashSet};
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
    /// Every spawn whose result nobody has taken yet, **callback or no callback**.
    ///
    /// Separate from `WAITING`, which holds only the ones that asked for `on_exit`. A join has to
    /// know when there is nothing left to wait for, and `oslo.spawn{"make","sub"}` with no callback
    /// is still work in flight — counting only the callbacks would make `oslo.settle()` answer
    /// immediately and report everything finished.
    static LIVE: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

/// What workers have finished, waiting to be handed back.
fn done() -> &'static Mutex<Vec<Finished>> {
    static DONE: OnceLock<Mutex<Vec<Finished>>> = OnceLock::new();
    DONE.get_or_init(|| Mutex::new(Vec::new()))
}

mod settle;

/// Add `oslo.spawn`.
pub fn install(oslo: &mut Table) {
    put(oslo, "spawn", |_, args| start(&args));
    settle::install(oslo);
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
    let on_exit = match spec.get_str("on_exit") {
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
    let timeout = match spec.get_str("timeout").as_number() {
        Some(n) if n.as_float() > 0.0 => Some(super::util::wait_from_millis(n.as_float())),
        _ => None,
    };

    let id = NEXT_ID.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next += 1;
        id
    });
    LIVE.with(|slot| slot.borrow_mut().insert(id));
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
    let mut table = super::handle::Handle::new("oslo.spawn");

    table.verb("cancel", move |_, _| {
        let had = WAITING.with(|slot| slot.borrow_mut().remove(&id).is_some());
        LIVE.with(|slot| slot.borrow_mut().remove(&id));
        ok(Value::Bool(had))
    });

    // job:wait([timeout_ms]) -> out, status. See [`settle`] for why this is not a sleep loop.
    table.verb("wait", move |_, args| settle::wait(id, &args));

    // **`<close>` cancels, and the collector does not.** A spawn is written for its callback and
    // its handle is usually dropped on the spot, so a `__gc` that cancelled would cancel almost
    // every spawn in the shell. Saying `local job <close> = oslo.spawn{…}` is asking for the work
    // to be scoped to the block, which is a different thing and a deliberate one.
    table.on_close("oslo.spawn.close", move || {
        WAITING.with(|slot| slot.borrow_mut().remove(&id));
    });

    table.build()
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

    // **Drained on a thread of its own, because `try_wait` does not read the pipe.** Polling for
    // exit while nothing empties stdout means a child that writes more than a pipe buffer — 64 KiB
    // here — blocks on the write, never exits, reaches the deadline and is killed. Every command
    // with more than a screenful to say "timed out" with nothing to show for it.
    let reading = child.stdout.take().map(|mut pipe| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut out = String::new();
            let _ = pipe.read_to_string(&mut out);
            out
        })
    });

    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            // **The status the command actually left**, rather than the 0 this used to answer for
            // every timed command — which made `oslo.spawn{…, timeout = n}` report success for a
            // build that failed.
            Ok(Some(done)) => break done.code().unwrap_or(-1),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // 124 is what `timeout(1)` answers, which is the convention a script would expect.
                // The reader is joined below either way: the kill closes the pipe, so it ends, and
                // whatever the command managed to say before the deadline is worth handing back.
                break 124;
            }
            // `ECHILD`: something reaped it first, which a shell with `SIGCHLD` ignored does
            // routinely. It means finished, not failed — the same case that bit `external::run`.
            Err(_) => break 0,
        }
    };
    let out = reading
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default();
    (out, status)
}

/// Hand back whatever finished, calling the callbacks that were waiting.
///
/// Called by the read loop where nothing is held. **The queue is emptied before anything runs**: a
/// callback may spawn again, and appending to a list being drained is how that becomes a deadlock on
/// the mutex it is already inside.
pub fn deliver() {
    deliver_counting();
}

/// The same, answering how many callbacks it called — which is what `oslo.settle` reports.
fn deliver_counting() -> usize {
    let finished: Vec<Finished> = match done().lock() {
        Ok(mut done) => std::mem::take(&mut *done),
        Err(_) => return 0,
    };
    let mut called = 0;
    for job in finished {
        LIVE.with(|slot| slot.borrow_mut().remove(&job.id));
        let Some(handler) = WAITING.with(|slot| slot.borrow_mut().remove(&job.id)) else {
            // Cancelled while it ran, or it never had a callback. Both are ordinary.
            continue;
        };
        let args = vec![Value::str(&job.out), Value::int(job.status as i64)];
        if let Err(problem) = crate::lua::engine::call_here(&handler, args) {
            eprintln!("oslo: spawn: {problem}");
        }
        called += 1;
    }
    called
}

/// Whether this spawn is still outstanding.
pub(crate) fn is_live(id: u64) -> bool {
    LIVE.with(|slot| slot.borrow().contains(&id))
}

/// How many spawns have not been accounted for yet.
pub(crate) fn outstanding() -> usize {
    LIVE.with(|slot| slot.borrow().len())
}

/// Take one job's result out of the queue, leaving everybody else's.
///
/// **Its callback still runs**, because `on_exit` promising "always" and then not firing for a job
/// somebody also joined is the kind of conditional contract nobody can hold in their head.
fn claim(id: u64) -> Option<(String, i32)> {
    let taken = match done().lock() {
        Ok(mut done) => done
            .iter()
            .position(|job| job.id == id)
            .map(|at| done.remove(at)),
        Err(_) => None,
    }?;
    LIVE.with(|slot| slot.borrow_mut().remove(&id));
    if let Some(handler) = WAITING.with(|slot| slot.borrow_mut().remove(&id)) {
        let args = vec![Value::str(&taken.out), Value::int(taken.status as i64)];
        if let Err(problem) = crate::lua::engine::call_here(&handler, args) {
            eprintln!("oslo: spawn: {problem}");
        }
    }
    Some((taken.out, taken.status))
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
