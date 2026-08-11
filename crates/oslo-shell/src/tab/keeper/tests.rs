use super::*;
use crate::tab::{dir, scratch, store};
use std::ffi::CString;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// Run a program in the tab instead of a shell — the keeper does not care which, and a test that
/// started a real shell would be testing the shell.
fn exec(program: &str, args: &[&str]) -> ! {
    let path = CString::new(program).expect("no NUL");
    let mut argv = vec![path.clone()];
    argv.extend(args.iter().map(|a| CString::new(*a).expect("no NUL")));
    let _ = nix::unistd::execv(&path, &argv);
    // Only reached if execv failed, and a child that cannot exec must not return anywhere.
    std::process::exit(127)
}

/// End a tab the way `tab -k` will: signal the keeper and wait for it, so the next test does not
/// inherit a process still holding a lock in a directory that is about to be deleted.
fn stop(keeper: nix::unistd::Pid) {
    let _ = nix::sys::signal::kill(keeper, nix::sys::signal::Signal::SIGTERM);
    let _ = nix::sys::wait::waitpid(keeper, None);
}

/// Make a tab whose shell is `run`, and answer with the keeper's pid.
///
/// `spawn` returns in two processes; a test only ever wants to be the one that asked, so the other
/// branch execs and never comes back here.
fn start(name: &str, run: impl FnOnce()) -> std::io::Result<nix::unistd::Pid> {
    match spawn(name, 0)? {
        Role::Caller(keeper) => Ok(keeper),
        Role::Inside => {
            run();
            std::process::exit(127)
        }
    }
}

/// Wait for something to become true, so a slow machine fails late rather than flakily.
fn until(what: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

/// **The whole mechanism, end to end**: a tab exists, something is running in it, and a client can
/// talk to that something over the socket.
#[test]
fn a_tab_runs_a_program_and_a_client_can_talk_to_it() {
    let (_scratch, _lock) = scratch();
    dir::open_checked().expect("tab directory");

    let keeper = start("alpha", || exec("/bin/cat", &[])).expect("spawn");
    assert!(until(|| store::alive("alpha")), "the tab never came up");
    assert!(
        until(|| store::Paths::new("alpha").sock().exists()),
        "no socket"
    );

    let mut client = std::os::unix::net::UnixStream::connect(store::Paths::new("alpha").sock())
        .expect("connect");
    client
        .write_all(&crate::tab::wire::data(b"hello\n"))
        .expect("write");
    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");

    let mut seen = Vec::new();
    let mut buffer = [0u8; 256];
    while !String::from_utf8_lossy(&seen).contains("hello") {
        match client.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.extend_from_slice(&buffer[..n]),
        }
    }
    assert!(
        String::from_utf8_lossy(&seen).contains("hello"),
        "nothing came back: {:?}",
        String::from_utf8_lossy(&seen)
    );

    // What the tab printed is on disk too, which is what `tail -f` reads and what a reattach
    // replays.
    let logged = std::fs::read(store::Paths::new("alpha").log()).unwrap_or_default();
    assert!(
        String::from_utf8_lossy(&logged).contains("hello"),
        "the log is empty"
    );
    stop(keeper);
}

/// The tab ends when what is running in it ends, and tidies up after itself.
#[test]
fn a_tab_ends_with_its_program() {
    let (_scratch, _lock) = scratch();
    dir::open_checked().expect("tab directory");

    start("beta", || exec("/bin/true", &[])).expect("spawn");
    assert!(
        until(|| !store::alive("beta")),
        "the tab outlived its program"
    );
    assert!(
        until(|| !store::Paths::new("beta").sock().exists()),
        "it left its socket behind"
    );
}

/// Two keepers for one name is the thing the lock exists to prevent.
#[test]
fn a_second_tab_of_the_same_name_is_refused() {
    let (_scratch, _lock) = scratch();
    dir::open_checked().expect("tab directory");

    let keeper = start("gamma", || exec("/bin/cat", &[])).expect("spawn");
    assert!(until(|| store::alive("gamma")), "the first never came up");

    let again = start("gamma", || exec("/bin/cat", &[]));
    assert!(again.is_err(), "a second keeper was allowed");
    stop(keeper);
}

/// A name that could escape the directory never reaches a fork.
#[test]
fn an_impossible_name_is_refused_before_anything_happens() {
    let (_scratch, _lock) = scratch();
    assert!(start("../escape", || exec("/bin/true", &[])).is_err());
    assert!(start("", || exec("/bin/true", &[])).is_err());
}

/// A resize reaches the pty, which is the only reason the socket is framed at all.
///
/// Asserted through a program that reports the size rather than by reading the ioctl back: what
/// matters is that something *inside* the tab sees it.
#[test]
fn a_resize_reaches_the_program_inside() {
    let (_scratch, _lock) = scratch();
    dir::open_checked().expect("tab directory");

    // `stty size` prints "rows cols" from whatever terminal it is standing on.
    let keeper = start("epsilon", || {
        exec("/bin/sh", &["-c", "sleep 0.4; stty size; sleep 5"])
    })
    .expect("spawn");
    assert!(
        until(|| store::Paths::new("epsilon").sock().exists()),
        "no socket"
    );

    let mut client = std::os::unix::net::UnixStream::connect(store::Paths::new("epsilon").sock())
        .expect("connect");
    client
        .write_all(&crate::tab::wire::resize(31, 121))
        .expect("resize");

    client
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");
    let mut seen = Vec::new();
    let mut buffer = [0u8; 256];
    while !String::from_utf8_lossy(&seen).contains("31 121") {
        match client.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => seen.extend_from_slice(&buffer[..n]),
        }
    }
    assert!(
        String::from_utf8_lossy(&seen).contains("31 121"),
        "the program inside saw {:?}",
        String::from_utf8_lossy(&seen)
    );
    stop(keeper);
}
