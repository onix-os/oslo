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
use oslo_shell::exec::redirect::SAVE_FD_FLOOR;
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

/// How long to wait after an `accept` error that may yet come right.
const RETRY_PAUSE: Duration = Duration::from_millis(100);

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
    start_at(wire::socket_path("oslo", None), env)
}

/// The same, at a path the caller names.
///
/// **The seam exists for the tests, and it exists because they were binding somebody's shell.**
/// `wire::socket_path` names the session, and a session id is *inherited* through
/// `$OSLO_SESSION` — so a test binary started from an oslo prompt computes the path of the shell
/// that started it. Two test processes then raced for one name and one of them got
/// `EADDRINUSE`, which is a failure about the developer's terminal and not about the server.
fn start_at(path: PathBuf, env: &Arc<Mutex<Environment>>) -> Result<PathBuf, String> {
    let mut bound = BOUND.lock().map_err(|_| "the server state is poisoned")?;
    if let Some(already) = bound.as_ref() {
        return Ok(already.path.clone());
    }

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

    sweep(dir);
    let listener =
        UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    let listener = above_the_scripts_range(listener);

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

/// Remove every socket in `dir` that nothing is listening on.
///
/// **A socket file left by a killed shell is not a running server**, and connecting is the only
/// test of that which cannot be raced — the file existing says nothing.
///
/// All of them, not only the name about to be bound. A name is a pid and a start time, so it never
/// comes round again and a leftover blocks nobody — but nothing removed one either, and this
/// directory had 488 dead sockets in it after a few days of killed shells. Only a shell that serves
/// puts one there, so a shell about to serve is exactly the right one to clear them.
fn sweep(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().is_some_and(|end| end == "sock") && UnixStream::connect(&path).is_err()
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn restrict(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
}

/// Move the listener out of the range a script may redirect.
///
/// **`bind` takes the lowest free descriptor, and that is usually 3.** A script writing `exec 3>log`
/// — or a parent process that put a file on descriptor 3 before `exec`ing the shell — then `dup2`s
/// over the listening socket, and this thread is left holding a number that is somebody else's file.
/// `accept` on it fails instantly and for ever. Four shells were doing 108,000 failed accepts a
/// second, each pinning a core, having served for a day beforehand with nothing to show it.
///
/// `procsub` moves its descriptors for the same reason and keeps a separate helper: it dups
/// *without* `FD_CLOEXEC` because the child has to inherit the result, where a listening socket
/// leaked into every command the shell runs is exactly what must not happen.
fn above_the_scripts_range(listener: UnixListener) -> UnixListener {
    use nix::fcntl::{FcntlArg, fcntl};
    use std::os::fd::{AsRawFd, FromRawFd};

    if listener.as_raw_fd() >= SAVE_FD_FLOOR {
        return listener;
    }
    match fcntl(
        listener.as_raw_fd(),
        FcntlArg::F_DUPFD_CLOEXEC(SAVE_FD_FLOOR),
    ) {
        // Dropping the original closes the low descriptor, which is the point.
        Ok(moved) => unsafe { UnixListener::from_raw_fd(moved) },
        // A listener on a low number still serves every peer that arrives; refusing to serve at
        // all because it could not be moved would be the worse of the two.
        Err(_) => listener,
    }
}

/// Whether an `accept` error says the *listener* is finished, rather than one connection.
///
/// `ENOTSOCK` is the one that happened, and `EBADF` is the same accident a step further along.
/// Neither can come right by trying again, so a loop that treats them as transient is a loop that
/// never stops.
fn listener_is_gone(e: &std::io::Error) -> bool {
    matches!(
        e.raw_os_error(),
        Some(nix::libc::ENOTSOCK | nix::libc::EBADF | nix::libc::EINVAL)
    )
}

fn accept_loop(listener: UnixListener, env: &Arc<Mutex<Environment>>, path: &Path) {
    let live = Arc::new(AtomicBool::new(true));
    let mut open = 0usize;
    for stream in listener.incoming() {
        if STOPPING.load(Ordering::SeqCst) {
            break;
        }
        let stream = match stream {
            Ok(stream) => stream,
            // The descriptor is not this server's socket any more. Nothing further will ever be
            // accepted on it, so the loop ends rather than asking again as fast as it can.
            Err(e) if listener_is_gone(&e) => break,
            // Anything else is about one connection, or is worth waiting out — `EMFILE` clears
            // when a descriptor is returned. A pause costs an idle server nothing and is the
            // difference between retrying and spinning.
            Err(_) => {
                std::thread::sleep(RETRY_PAUSE);
                continue;
            }
        };

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

/// One at a time: the socket path is per *process*, so two tests that bind would fight over one
/// name — and so would one that binds and one that asserts nothing is bound.
///
/// **Shared with `super::tests`, which is why it lives out here.** Each module having its own
/// mutex is two mutexes and no exclusion at all: `a_shell_that_never_served_holds_no_snapshot`
/// failed about one run in five because `publish` does nothing unless something is bound, and
/// these tests were what bound it.
#[cfg(test)]
pub(super) fn serialised() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: Mutex<()> = Mutex::new(());
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// [`start`] at a name no other process can compute — see [`start_at`] for why the real one will
/// not do.
#[cfg(test)]
pub(super) fn serve(env: &Arc<Mutex<Environment>>) -> Result<PathBuf, String> {
    let path = wire::socket_path("oslo-test", None)
        .with_file_name(format!("t{}.sock", std::process::id()));
    start_at(path, env)
}

/// Starting, stopping and starting again — the sequence a toggle keybinding produces.
#[cfg(test)]
mod tests {
    use super::*;

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

        let first = serve(&env).expect("it binds");
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
        let second = serve(&env).expect("it binds again");
        assert_eq!(second, first, "the same session, so the same name");
        assert!(
            UnixStream::connect(&second).is_ok(),
            "the second server answers rather than having had its socket deleted"
        );

        assert!(stop());
    }

    /// **A killed shell leaves its socket behind, and nothing used to remove it.** 488 of them had
    /// piled up in the runtime directory here. A shell that serves is the one that put one there,
    /// so it is the one that clears the dead ones — and "dead" is decided by connecting, which is
    /// the only test of it that cannot be raced.
    #[test]
    fn serving_clears_the_sockets_nobody_is_listening_on() {
        let _serial = serialised();
        let env = shell();

        let live = serve(&env).expect("it binds");
        let dir = live
            .parent()
            .expect("a socket has a directory")
            .to_path_buf();
        let dead = dir.join("t0-abandoned.sock");
        std::fs::write(&dead, b"").expect("leave one behind");
        assert!(stop());

        // The next serve is what sweeps: the abandoned one goes, and the one being bound answers.
        let again = serve(&env).expect("it binds again");
        assert!(!dead.exists(), "the abandoned socket is still there");
        assert!(UnixStream::connect(&again).is_ok(), "and this one serves");
        assert!(stop());
    }

    /// Asking twice is what a keybinding pressed twice does, and the honest answer is the path it
    /// is already serving rather than a second listener on a name that can only have one.
    #[test]
    fn serving_twice_answers_the_same_path() {
        let _serial = serialised();
        let env = shell();

        let first = serve(&env).expect("binds");
        let again = serve(&env).expect("answers the same");
        assert_eq!(first, again);
        assert!(UnixStream::connect(&first).is_ok());

        assert!(stop());
        assert!(!stop(), "stopping what is not running says so");
    }

    /// A descriptor in the script's range that nothing currently has open.
    fn a_free_low_descriptor() -> Option<i32> {
        use nix::fcntl::{FcntlArg, fcntl};
        (3..SAVE_FD_FLOOR).find(|fd| fcntl(*fd, FcntlArg::F_GETFD).is_err())
    }

    /// **The listener must not sit on a number a script may redirect.** `bind` takes the lowest
    /// free descriptor, which is usually 3, and `exec 3>log` then puts a regular file there.
    #[test]
    fn a_listener_does_not_sit_where_a_redirect_can_reach_it() {
        use std::os::fd::{AsRawFd, FromRawFd};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("control.sock");
        let bound = UnixListener::bind(&path).expect("bind");

        let Some(low) = a_free_low_descriptor() else {
            return;
        };
        nix::unistd::dup2(bound.as_raw_fd(), low).expect("dup2");
        drop(bound);
        let listener = unsafe { UnixListener::from_raw_fd(low) };

        let moved = above_the_scripts_range(listener);
        assert!(
            moved.as_raw_fd() >= SAVE_FD_FLOOR,
            "left on {}, which `exec {}>file` overwrites",
            moved.as_raw_fd(),
            moved.as_raw_fd()
        );

        // Still a working listener, rather than merely a different number.
        let reaching = path.clone();
        let peer = std::thread::spawn(move || UnixStream::connect(&reaching).is_ok());
        assert!(moved.accept().is_ok(), "the moved descriptor still accepts");
        assert!(peer.join().expect("peer"), "and the peer got through");
    }

    /// The errors that mean the listener is finished, told apart from the ones about a single peer.
    #[test]
    fn only_a_finished_listener_ends_the_loop() {
        let gone = |code| listener_is_gone(&std::io::Error::from_raw_os_error(code));

        assert!(gone(nix::libc::ENOTSOCK), "the one that burned a core");
        assert!(gone(nix::libc::EBADF));

        assert!(!gone(nix::libc::ECONNABORTED), "one peer, not the socket");
        assert!(!gone(nix::libc::EINTR));
        assert!(
            !gone(nix::libc::EMFILE),
            "clears when a descriptor comes back"
        );
    }

    /// **The regression.** A regular file `dup2`'d over the socket made every later `accept` fail
    /// with `ENOTSOCK` at once, and the loop asked again as fast as the kernel could answer: four
    /// shells were each doing 108,000 failed accepts a second, having served correctly for a day
    /// beforehand.
    ///
    /// The loop has to *end*. Run on a thread so that a regression is a failing test rather than a
    /// suite that never finishes.
    #[test]
    fn a_loop_whose_socket_was_replaced_ends_instead_of_spinning() {
        use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("control.sock");
        // The descriptor is taken out of the listener's ownership first, so that replacing it is a
        // redirect rather than a double close.
        let fd = UnixListener::bind(&path).expect("bind").into_raw_fd();

        // Exactly what a redirect on the listener's number does to it.
        let file = std::fs::File::create(dir.path().join("log")).expect("file");
        nix::unistd::dup2(file.as_raw_fd(), fd).expect("dup2");
        let listener = unsafe { UnixListener::from_raw_fd(fd) };

        let env = Arc::new(Mutex::new(Environment::new()));
        let (tell, told) = std::sync::mpsc::channel();
        let where_ = path.clone();
        std::thread::spawn(move || {
            accept_loop(listener, &env, &where_);
            let _ = tell.send(());
        });

        assert!(
            told.recv_timeout(Duration::from_secs(10)).is_ok(),
            "the accept loop never returned, which is the spin"
        );
    }
}
