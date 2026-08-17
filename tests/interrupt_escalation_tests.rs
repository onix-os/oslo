//! Ctrl-C three times kills a foreground job that will not die of one.
//!
//! **A real pty, because there is nowhere else this exists.** The whole mechanism is about which
//! process group the terminal driver signals, and a pipe has no terminal driver — a test that drove
//! oslo through a pipe would be testing nothing at all.
//!
//! Each case runs a job that `trap`s `INT` away, so the ordinary interrupt provably does not end
//! it, and then counts keystrokes. The marker in the job's command line is unique per case so that
//! `pgrep` cannot see another case's leftovers.

mod common;

use nix::pty::openpty;
use std::fs;
use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn owned(fd: OwnedFd) -> fs::File {
    fs::File::from(fd)
}

struct Shell {
    input: fs::File,
    seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    _home: tempfile::TempDir,
    child: std::process::Child,
}

impl Shell {
    fn start() -> Shell {
        let pty = openpty(None, None).expect("open pty");
        let master = owned(pty.master);
        let slave = owned(pty.slave);
        let home = tempfile::tempdir().expect("temporary home");
        let dir = home.path().join("config/oslo");
        fs::create_dir_all(&dir).expect("config directory");
        fs::write(dir.join("init.lua"), "oslo.misc.welcome = false\n").expect("config");

        let mut command = Command::new(common::oslo_bin());
        command
            .arg("-i")
            .env_clear()
            .env("HOME", home.path())
            .env("XDG_DATA_HOME", home.path())
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .current_dir(home.path())
            .stdin(Stdio::from(slave.try_clone().expect("clone")))
            .stdout(Stdio::from(slave.try_clone().expect("clone")))
            .stderr(Stdio::from(slave.try_clone().expect("clone")));
        // SAFETY: runs after fork and calls only async-signal-safe interfaces.
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
        let child = command.spawn().expect("spawn oslo on a pty");
        drop(slave);

        let input = master.try_clone().expect("clone master");
        let seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>> = Default::default();
        let filling = std::sync::Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 4096];
            while let Ok(n) = std::io::Read::read(&mut (&master), &mut buffer) {
                if n == 0 {
                    break;
                }
                filling
                    .lock()
                    .expect("transcript")
                    .extend_from_slice(&buffer[..n]);
            }
        });
        Shell {
            input,
            seen,
            _home: home,
            child,
        }
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().expect("transcript")).into_owned()
    }

    /// Wait for evidence rather than a duration — a debug build under a loaded suite is slow to
    /// reach its first prompt, and a passing case returns the moment its evidence appears.
    fn until(&self, what: impl Fn(&str) -> bool) -> bool {
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline {
            if what(&self.text()) {
                return true;
            }
            sleep(Duration::from_millis(25));
        }
        false
    }

    fn type_line(&mut self, line: &str) {
        self.input.write_all(line.as_bytes()).expect("write");
        self.input.write_all(b"\n").expect("write");
    }

    /// Send the interrupt character, which the terminal driver turns into a `SIGINT` for whichever
    /// process group currently owns the terminal.
    fn interrupt(&mut self) {
        self.input.write_all(b"\x03").expect("write");
        self.input.flush().expect("flush");
        // Far enough apart that the three are three deliveries and not one coalesced one.
        sleep(Duration::from_millis(400));
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Start a job that ignores `INT` and `TERM`, carrying `marker` in its command line.
fn stubborn(shell: &mut Shell, marker: &str) {
    shell.type_line(&format!(
        "sh -c 'trap \"\" INT TERM; echo READY-{marker}; sleep {marker}'"
    ));
    assert!(
        shell.until(|seen| seen.contains(&format!("READY-{marker}"))),
        "the job never started: {}",
        shell.text()
    );
    sleep(Duration::from_millis(400));
}

/// Whether a process carrying `marker` is still running.
fn running(marker: &str) -> bool {
    Command::new("pgrep")
        .arg("-f")
        .arg(format!("sleep {marker}"))
        .output()
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(false)
}

fn cleanup(marker: &str) {
    let _ = Command::new("pkill")
        .arg("-9")
        .arg("-f")
        .arg(format!("sleep {marker}"))
        .status();
}

/// **The point.** Three interrupts end a job that will not end for one, and the shell is still
/// there afterwards — which is the half that makes it a feature rather than a way to lose a shell.
#[test]
fn three_interrupts_kill_a_job_that_ignores_them() {
    let marker = "3111";
    let mut shell = Shell::start();
    assert!(shell.until(|seen| seen.contains('$') || seen.contains('>')));
    stubborn(&mut shell, marker);

    for _ in 0..3 {
        shell.interrupt();
    }
    // The escalation is a `SIGKILL` to the group; give the kernel a moment to reap it.
    let gone = (0..40).any(|_| {
        if !running(marker) {
            return true;
        }
        sleep(Duration::from_millis(100));
        false
    });
    let alive_shell = {
        shell.type_line("echo STILL''-HERE");
        shell.until(|seen| seen.contains("STILL-HERE"))
    };
    cleanup(marker);

    assert!(gone, "the job survived three interrupts:\n{}", shell.text());
    assert!(
        alive_shell,
        "the shell did not survive the escalation:\n{}",
        shell.text()
    );
}

/// **Two is not three.** A job that ignores the interrupt is still running after two, because the
/// escalation is something you have to mean.
#[test]
fn two_interrupts_leave_the_job_alone() {
    let marker = "3222";
    let mut shell = Shell::start();
    assert!(shell.until(|seen| seen.contains('$') || seen.contains('>')));
    stubborn(&mut shell, marker);

    shell.interrupt();
    shell.interrupt();
    sleep(Duration::from_millis(800));

    let still_there = running(marker);
    cleanup(marker);
    assert!(
        still_there,
        "two interrupts killed a job that traps them:\n{}",
        shell.text()
    );
}

/// **The ordinary case is unchanged**: a job that does not trap the interrupt still dies of the
/// first one, and the shell comes straight back. The escalation must not have made Ctrl-C slower.
#[test]
fn one_interrupt_still_ends_an_ordinary_job() {
    let marker = "3333";
    let mut shell = Shell::start();
    assert!(shell.until(|seen| seen.contains('$') || seen.contains('>')));

    shell.type_line(&format!("sh -c 'echo READY-{marker}; sleep {marker}'"));
    assert!(
        shell.until(|seen| seen.contains(&format!("READY-{marker}"))),
        "the job never started: {}",
        shell.text()
    );
    sleep(Duration::from_millis(400));

    shell.interrupt();
    let gone = (0..40).any(|_| {
        if !running(marker) {
            return true;
        }
        sleep(Duration::from_millis(100));
        false
    });
    shell.type_line("echo BACK''-AT-IT");
    let back = shell.until(|seen| seen.contains("BACK-AT-IT"));
    cleanup(marker);

    assert!(
        gone,
        "one interrupt no longer ends a job:\n{}",
        shell.text()
    );
    assert!(back, "the prompt did not return:\n{}", shell.text());
}
