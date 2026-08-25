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

/// Whether a socket is bound, and where.
static BOUND: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Set to ask the accept loop to stop. Read after every accept, so a wake is all it takes.
static STOPPING: AtomicBool = AtomicBool::new(false);

/// Bind the socket and start serving. Answers the path, or why not.
///
/// **Idempotent.** Asking twice is what a keybinding does when somebody presses it twice, and the
/// honest answer is the path it is already serving rather than a second listener on a name that can
/// only have one.
pub fn start(env: &Arc<Mutex<Environment>>) -> Result<PathBuf, String> {
    let mut bound = BOUND.lock().map_err(|_| "the server state is poisoned")?;
    if let Some(already) = bound.as_ref() {
        return Ok(already.clone());
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
    std::thread::Builder::new()
        .name("oslo-live".to_string())
        .spawn(move || accept_loop(listener, &served, &where_))
        .map_err(|e| format!("could not start the server thread: {e}"))?;

    *bound = Some(path.clone());
    Ok(path)
}

/// Stop serving and remove the socket. Answers whether one was running.
pub fn stop() -> bool {
    let Ok(mut bound) = BOUND.lock() else {
        return false;
    };
    let Some(path) = bound.take() else {
        return false;
    };
    STOPPING.store(true, Ordering::SeqCst);
    // The accept is blocking, so it has to be woken to notice. Connecting to ourselves is the
    // wake: the loop accepts it, reads the flag and leaves.
    let _ = UnixStream::connect(&path);
    let _ = std::fs::remove_file(&path);
    true
}

/// The path being served, if any.
pub fn serving() -> Option<PathBuf> {
    BOUND.lock().ok().and_then(|bound| bound.clone())
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
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(call, &args, env, LOCK_WAIT)
    })) {
        Ok(Ok(result)) => Reply::ok(result),
        Ok(Err(why)) => Reply::failed(&why),
        Err(_) => Reply::failed(&format!("{call} failed")),
    }
}
