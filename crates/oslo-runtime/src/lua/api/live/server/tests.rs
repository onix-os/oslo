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
/// A duplicate of `fd` on the lowest free descriptor at or above 3, if that is still inside
/// the range a script may redirect.
///
/// `F_DUPFD` picks and claims in one step. Looking for a free number and *then* taking it is a
/// race another thread wins by opening anything — which is `EBUSY`, and which a test that
/// opens sockets alongside this one duly provoked.
fn a_duplicate_in_the_scripts_range(fd: i32) -> Option<i32> {
    use nix::fcntl::{FcntlArg, fcntl};
    match fcntl(fd, FcntlArg::F_DUPFD(3)) {
        Ok(low) if low < SAVE_FD_FLOOR => Some(low),
        Ok(high) => {
            let _ = nix::unistd::close(high);
            None
        }
        Err(_) => None,
    }
}

/// **The listener must not sit on a number a script may redirect.** `bind` takes the lowest
/// free descriptor, which is usually 3, and `exec 3>log` then puts a regular file there.
#[test]
fn a_listener_does_not_sit_where_a_redirect_can_reach_it() {
    use std::os::fd::{AsRawFd, FromRawFd};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("control.sock");
    let bound = UnixListener::bind(&path).expect("bind");

    let Some(low) = a_duplicate_in_the_scripts_range(bound.as_raw_fd()) else {
        return;
    };
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

/// Ask one question over a fresh connection. `None` if the server never answered.
fn ask(path: &Path, call: &str) -> Option<String> {
    let mut stream = UnixStream::connect(path).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;

    let body = format!("{{\"call\":\"{call}\",\"args\":[]}}");
    stream.write_all(&wire::header(body.len())).ok()?;
    stream.write_all(body.as_bytes()).ok()?;
    stream.flush().ok()?;

    let mut head = [0u8; wire::HEADER];
    stream.read_exact(&mut head).ok()?;
    let mut reply = vec![0u8; wire::body_len(head)?];
    stream.read_exact(&mut reply).ok()?;
    String::from_utf8(reply).ok()
}

/// **A server that once hit its connection cap still answers afterwards.**
///
/// The slot count was reclaimed in the loop's *last* statement, which the two `continue`s
/// above it skipped. So the peer refused for being over the cap took the branch that never
/// refreshes the count — `open` stuck at `MAX_CONNS` with nothing able to lower it again, and
/// the shell refused every caller for the rest of its life.
///
/// It has to be concurrent to show: served one at a time the reclaim at the bottom was reached
/// every iteration and the count never wedged, so a serial version of this test passes on the
/// broken code.
#[test]
fn a_server_that_once_hit_its_cap_still_answers() {
    let _serial = serialised();
    let env = shell();
    let path = serve(&env).expect("it binds");

    // Held open, so the server really is at its cap rather than merely having been busy.
    let held: Vec<UnixStream> = (0..MAX_CONNS)
        .filter_map(|_| UnixStream::connect(&path).ok())
        .collect();
    assert_eq!(held.len(), MAX_CONNS, "every one connected");
    std::thread::sleep(Duration::from_millis(200));

    // The one over the cap: the iteration that used to skip the reclaim.
    let refused = UnixStream::connect(&path);
    assert!(
        refused.is_ok(),
        "connecting still succeeds; serving is what is refused"
    );
    drop(refused);
    drop(held);

    // Now that every slot is genuinely free, a caller must be served again. The retry is for
    // the connection threads to finish, not for the count: on the broken code no number of
    // retries ever succeeds.
    let answered = (0..50).any(|_| {
        if ask(&path, "session").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
        false
    });
    assert!(
        answered,
        "the server never answered again after reaching its cap"
    );

    assert!(stop());
}
