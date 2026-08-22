//! A window resize has to reach the editor from *either* of its two waits.
//!
//! The editor waits two ways: a blocking read when nothing is pending, and a timed one while an
//! idle hook is armed or a prompt is being rebuilt off the thread. `SIGWINCH` interrupts the poll
//! in both — but the timed wait reported the `EINTR` as a plain timeout, so the flag stayed set and
//! the line was never laid out again until the next keystroke.
//!
//! That is what made losing the prompt on resize feel random: it depended entirely on which of the
//! two waits the editor happened to be sitting in when the window changed.

mod common;

use nix::pty::openpty;
use std::fs;
use std::io::Write;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn owned(fd: OwnedFd) -> fs::File {
    fs::File::from(fd)
}

/// Set the pty's window size, which is what raises `SIGWINCH` in the child.
fn set_size(fd: i32, cols: u16, rows: u16) {
    let size = nix::libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `fd` is a live pty master and `size` is a fully initialised `winsize`.
    unsafe {
        nix::libc::ioctl(fd, nix::libc::TIOCSWINSZ, &size);
    }
}

/// How many bytes an interactive oslo writes when the window is resized and nothing is typed.
///
/// `config` decides which wait it is sitting in.
fn bytes_redrawn_on_resize(config: &str) -> usize {
    let pty = openpty(None, None).expect("open pty");
    let master = owned(pty.master);
    let slave = owned(pty.slave);
    let home = tempfile::tempdir().expect("temporary home");
    if !config.is_empty() {
        let dir = home.path().join("config/oslo");
        fs::create_dir_all(&dir).expect("config directory");
        fs::write(dir.join("init.lua"), config).expect("config");
    }

    let master_fd = master.as_raw_fd();
    set_size(master_fd, 100, 40);

    let mut command = Command::new(common::oslo_bin());
    command
        .arg("-i")
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .current_dir(home.path())
        .stdin(Stdio::from(slave.try_clone().expect("clone")))
        .stdout(Stdio::from(slave.try_clone().expect("clone")))
        .stderr(Stdio::from(slave.try_clone().expect("clone")));
    // SAFETY: runs after fork and calls only async-signal-safe system interfaces.
    unsafe {
        command.pre_exec(|| {
            if nix::libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if nix::libc::ioctl(0, nix::libc::TIOCSCTTY, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().expect("spawn oslo on a pty");
    drop(slave);

    let seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = Default::default();
    let filling = std::sync::Arc::clone(&seen);
    let reader = master.try_clone().expect("clone master");
    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        while let Ok(n) = std::io::Read::read(&mut (&reader), &mut buffer) {
            if n == 0 {
                break;
            }
            filling
                .lock()
                .expect("transcript")
                .extend_from_slice(&buffer[..n]);
        }
    });

    // Wait for the first prompt rather than a fixed duration: a debug build loading a Lua config is
    // slow to start when the rest of the suite is running beside it.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if !seen.lock().expect("transcript").is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    std::thread::sleep(Duration::from_millis(600));

    seen.lock().expect("transcript").clear();
    set_size(master_fd, 40, 40);
    std::thread::sleep(Duration::from_millis(1200));
    let redrawn = seen.lock().expect("transcript").len();

    let mut input = master.try_clone().expect("clone master");
    let _ = input.write_all(b"\x15exit\n");
    std::thread::sleep(Duration::from_millis(200));
    let _ = child.kill();
    let _ = child.wait();
    redrawn
}

/// **The case that was already fixed**, kept so the two halves are tested together: with nothing
/// pending the editor blocks, and the marker reaches it.
#[test]
fn a_resize_redraws_from_the_blocking_wait() {
    assert!(
        bytes_redrawn_on_resize("") > 0,
        "a resize with nothing pending must lay the line out again"
    );
}

/// **The half that was lost.** An armed idle hook puts the editor in its *timed* wait, where
/// `SIGWINCH` arrived as an `EINTR` that was reported as a plain timeout — so the resize sat in its
/// flag and nothing was redrawn until a key was pressed.
#[test]
fn a_resize_redraws_from_the_timed_wait() {
    let config = "oslo.misc.idle_timeout = 300\noslo.on.idle_timeout(function(i) end)\n";
    assert!(
        bytes_redrawn_on_resize(config) > 0,
        "a resize while an idle hook is armed must lay the line out again — \
         this is the one that made losing the prompt on resize feel random"
    );
}
