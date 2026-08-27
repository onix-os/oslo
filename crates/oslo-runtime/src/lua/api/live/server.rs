//! The control socket: accepting, framing, and who is allowed to speak.
//!
//! # It runs on a thread, and that is a consequence of the surface rather than a design goal
//!
//! Every verb in [`super::VERBS`] answers from the [`Environment`], which is behind an
//! `Arc<Mutex<…>>` and is `Send`. **None of them needs the Lua VM.** So the whole server lives on a
//! thread of its own with a clone of that handle, and the read loop never learns it exists.
//!
//! That is worth stating because the alternative is a great deal worse. A surface that needed Lua
//! would need the main thread — the VM is not `Send` — which means a queue, a drain at prompt time,
//! and an answer to what happens when a request arrives while the config is mid-call. Choosing
//! verbs that answer from the environment bought all of that away.
//!
//! Adding a verb that needs Lua therefore is not a small change. It is this file becoming a queue.
//!
//! # Nothing here may make the shell wait
//!
//! The environment lock is taken with `try_lock` and a short retry. A foreground command holds it,
//! and a server thread that blocked on it would sit there for as long as `cargo build` takes while
//! the client's own deadline expired anyway. "The shell is busy" is a true answer that arrives.

use super::{Reply, dispatch};
use oslo_base::wire;
use oslo_shell::env::Environment;
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Connections served at once. A control socket answers occasional questions; a peer that opens
/// more than this is misbehaving and is better refused than queued.
const MAX_CONNS: usize = 8;

/// How long one connection may stay open with nothing arriving.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long to keep trying for the environment lock before answering that the shell is busy.
const LOCK_WAIT: Duration = Duration::from_millis(200);

/// What is being served: where, and the thread doing it.
///
/// **The thread is kept so [`stop`] can join it**, which is what makes stopping and starting again
/// safe. Without the handle, `stop` set the flag, poked the accept awake and returned — so a
/// `stop(); serve()` on one line, or a toggle keybinding pressed twice, raced two ways. Either
/// `start` cleared the flag before the old loop had read it, and that loop went back to sleep in
/// `incoming()` for the rest of the session holding a thread and a listening descriptor; or the old
/// loop reached its last act *after* the new bind and removed the **new** socket, leaving `serving()`
/// reporting a path that every client gets `ENOENT` from.
struct Serving {
    path: PathBuf,
    thread: std::thread::JoinHandle<()>,
}

/// Whether a socket is bound, and where.
static BOUND: Mutex<Option<Serving>> = Mutex::new(None);

/// Set to ask the accept loop to stop. Read after every accept, so a wake is all it takes.
static STOPPING: AtomicBool = AtomicBool::new(false);

/// How many accept loops have finished, for the one thing worth asserting about `stop`: that when
/// it returns, the loop it stopped is *over*. Everything the un-joined version got wrong followed
/// from that not being true.
static LOOPS_ENDED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Bind the socket and start serving. Answers the path, or why not.
///
/// **Idempotent.** Asking twice is what a keybinding does when somebody presses it twice, and the
/// honest answer is the path it is already serving rather than a second listener on a name that can
/// only have one.
pub fn start(env: &Arc<Mutex<Environment>>) -> Result<PathBuf, String> {
    let mut bound = BOUND.lock().map_err(|_| "the server state is poisoned")?;
    if let Some(already) = bound.as_ref() {
        return Ok(already.path.clone());
    }

    let path = wire::socket_path("oslo", None);
    if wire::too_long(&path) {
        return Err(format!(
            "the socket path is {} bytes and a unix address holds {}: {}",
            path.as_os_str().len(),
            wire::MAX_SOCKET_PATH,
            path.display()
        ));
    }
    let dir = path.parent().ok_or("the socket path has no directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // **0700, whatever the umask says.** The directory is the whole access control: a socket inside
    // one nobody else may traverse cannot be connected to by another user, and that holds even
    // where `$XDG_RUNTIME_DIR` is somewhere unusual.
    restrict(dir);

    // A socket file left by a killed shell is not a running server. Connecting is the only test
    // that cannot be raced — the file existing says nothing.
    if path.exists() && UnixStream::connect(&path).is_err() {
        let _ = std::fs::remove_file(&path);
    }
    let listener =
        UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;

    STOPPING.store(false, Ordering::SeqCst);
    // Before the thread exists, so a peer that connects instantly has somewhere to leave a `cd`.
    // A shell whose wake pipe could not be opened still serves every read verb; only the ones that
    // ask it to *do* something answer that they cannot.
    super::queued::arm();
    let served = Arc::clone(env);
    let where_ = path.clone();
    let thread = std::thread::Builder::new()
        .name("oslo-live".to_string())
        .spawn(move || accept_loop(listener, &served, &where_))
        .map_err(|e| format!("could not start the server thread: {e}"))?;

    *bound = Some(Serving {
        path: path.clone(),
        thread,
    });
    Ok(path)
}

/// Stop serving and remove the socket. Answers whether one was running.
pub fn stop() -> bool {
    let Ok(mut bound) = BOUND.lock() else {
        return false;
    };
    let Some(serving) = bound.take() else {
        return false;
    };
    STOPPING.store(true, Ordering::SeqCst);
    // The accept is blocking, so it has to be woken to notice. Connecting to ourselves is the
    // wake: the loop accepts it, reads the flag and leaves.
    let _ = UnixStream::connect(&serving.path);
    // **Joined before this returns**, which is the whole fix: `start` clears `STOPPING`, so a serve
    // that began while the old loop had not yet read it left that loop asleep in `incoming()` for
    // the session — and a loop that woke *after* the new bind removed the new socket on its way
    // out. Waiting here means there is never more than one loop, and the file below is always this
    // server's own.
    let _ = serving.thread.join();
    let _ = std::fs::remove_file(&serving.path);
    true
}

/// The path being served, if any.
pub fn serving() -> Option<PathBuf> {
    BOUND
        .lock()
        .ok()
        .and_then(|bound| bound.as_ref().map(|serving| serving.path.clone()))
}

fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

fn accept_loop(listener: UnixListener, env: &Arc<Mutex<Environment>>, path: &Path) {
    let live = Arc::new(AtomicBool::new(true));
    let mut open = 0usize;
    for stream in listener.incoming() {
        if STOPPING.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };

        // The uid the kernel reports, not one the peer sent. The directory mode should already
        // make this unreachable; it is here because "should" is not an access control.
        if !same_user(&stream) {
            continue;
        }
        if open >= MAX_CONNS {
            continue;
        }

        open += 1;
        let env = Arc::clone(env);
        let ticket = Arc::clone(&live);
        // One thread per connection, capped above. A connection is a handful of small calls, so
        // this costs a stack for as long as somebody is asking and nothing when nobody is.
        let spawned = std::thread::Builder::new()
            .name("oslo-live-conn".to_string())
            .spawn(move || {
                serve_connection(stream, &env);
                drop(ticket);
            });
        if spawned.is_err() {
            open -= 1;
        }
        // Reclaim slots from connections that have finished. `Arc::strong_count` is the cheap
        // approximation: one for the loop's own handle, one per live connection.
        open = Arc::strong_count(&live).saturating_sub(1).min(open);
    }
    let _ = std::fs::remove_file(path);
    LOOPS_ENDED.fetch_add(1, Ordering::SeqCst);
}

/// Whether the connecting process belongs to the same user.
fn same_user(stream: &UnixStream) -> bool {
    use nix::sys::socket::{getsockopt, sockopt};
    match getsockopt(&stream.as_fd(), sockopt::PeerCredentials) {
        Ok(peer) => peer.uid() == nix::unistd::getuid().as_raw(),
        // A kernel that will not answer is not a reason to let a stranger in.
        Err(_) => false,
    }
}

fn serve_connection(mut stream: UnixStream, env: &Arc<Mutex<Environment>>) {
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IDLE_TIMEOUT));

    // Several calls per connection: the client library holds one open and asks as it needs to,
    // which is cheaper than a connect per question and is what a keep-alive is for.
    loop {
        let Some(body) = read_frame(&mut stream) else {
            return;
        };
        let reply = answer(&body, env);
        if write_frame(&mut stream, reply.as_bytes()).is_err() {
            return;
        }
    }
}

fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
    let mut head = [0u8; wire::HEADER];
    stream.read_exact(&mut head).ok()?;
    let len = wire::body_len(head)?;
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).ok()?;
    Some(body)
}

fn write_frame(stream: &mut UnixStream, body: &[u8]) -> std::io::Result<()> {
    stream.write_all(&wire::header(body.len()))?;
    stream.write_all(body)?;
    stream.flush()
}

/// One request in, one reply out, and never a panic across the boundary.
fn answer(body: &[u8], env: &Arc<Mutex<Environment>>) -> String {
    let request: serde_json::Value = match serde_json::from_slice(body) {
        Ok(parsed) => parsed,
        Err(e) => return Reply::failed(&format!("the request is not JSON: {e}")),
    };
    let Some(call) = request.get("call").and_then(serde_json::Value::as_str) else {
        return Reply::failed("the request names no call");
    };
    let args = request
        .get("args")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();

    // A verb that panics must answer, not take the shell's server down with it: this thread dying
    // silently would leave a socket that accepts and never replies.
    //
    // **Inert in a release build**, where `panic = "abort"` means there is nothing to catch — see
    // the note on that setting. It still isolates in a debug build and under `cargo test`, which is
    // where a verb's panic is actually likely to be met.
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(call, &args, env, LOCK_WAIT)
    })) {
        Ok(Ok(result)) => Reply::ok(result),
        Ok(Err(why)) => Reply::failed(&why),
        Err(_) => Reply::failed(&format!("{call} failed")),
    }
}

/// Starting, stopping and starting again — the sequence a toggle keybinding produces.
#[cfg(test)]
mod tests {
    use super::*;

    /// One at a time: the socket path is per *process*, so two of these would fight over one name.
    fn serialised() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn shell() -> Arc<Mutex<Environment>> {
        Arc::new(Mutex::new(Environment::new()))
    }

    /// **A stop finishes before it returns, so the next serve is not racing it.**
    ///
    /// `stop` used to set the flag, poke the accept awake and return without waiting. `start` then
    /// cleared the flag, and the sequence raced two ways: the old loop could read the cleared flag
    /// and go back to sleep in `incoming()` for the rest of the session — a leaked thread and a
    /// listening descriptor — or it could reach its last act *after* the new bind and delete the
    /// **new** socket, leaving `serving()` reporting a path every client gets `ENOENT` from.
    ///
    /// Both show up the same way from outside: after `stop(); serve()`, is the socket connectable?
    #[test]
    fn stopping_and_serving_again_leaves_a_working_socket() {
        let _serial = serialised();
        let env = shell();

        let first = start(&env).expect("it binds");
        assert!(UnixStream::connect(&first).is_ok(), "the first one answers");
        assert_eq!(serving().as_ref(), Some(&first));

        // **The invariant the join establishes**, and the one the timing-dependent races all
        // followed from: when `stop` returns, the loop it stopped is over. Without the wait it
        // returned while that loop was still deciding, which is how one could go back to sleep for
        // the session, or wake after the next bind and delete a socket it no longer owned.
        let ended = LOOPS_ENDED.load(Ordering::SeqCst);
        assert!(stop(), "it was running");
        assert_eq!(
            LOOPS_ENDED.load(Ordering::SeqCst),
            ended + 1,
            "the accept loop had finished before stop returned"
        );
        assert_eq!(serving(), None, "and says so");
        assert!(
            UnixStream::connect(&first).is_err(),
            "the socket is gone once stop returns"
        );

        // The toggle, immediately — this is the sequence that raced.
        let second = start(&env).expect("it binds again");
        assert_eq!(second, first, "the same session, so the same name");
        assert!(
            UnixStream::connect(&second).is_ok(),
            "the second server answers rather than having had its socket deleted"
        );

        assert!(stop());
    }

    /// Asking twice is what a keybinding pressed twice does, and the honest answer is the path it
    /// is already serving rather than a second listener on a name that can only have one.
    #[test]
    fn serving_twice_answers_the_same_path() {
        let _serial = serialised();
        let env = shell();

        let first = start(&env).expect("binds");
        let again = start(&env).expect("answers the same");
        assert_eq!(first, again);
        assert!(UnixStream::connect(&first).is_ok());

        assert!(stop());
        assert!(!stop(), "stopping what is not running says so");
    }
}
