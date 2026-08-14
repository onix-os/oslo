//! Does an *idle* prompt notice a background job finishing?
//!
//! Before this, no: the editor is blocked in `read` on the terminal, a child exiting is not a
//! keystroke, and the shell learned about it at the next command boundary — which for somebody
//! sitting at a prompt means "when you next press Enter". A job that ended ten minutes ago was
//! announced when you ran the next thing.
//!
//! `SIGCHLD` is now installed without `SA_RESTART`, so the blocked `read` fails with `EINTR` and
//! the reader services the background before going back to waiting. The same route `SIGWINCH`
//! already takes for a resize.
//!
//! **A real pty, because that is the only place the editor exists.** A pipe is not interactive, the
//! line editor never runs, and the thing under test is precisely what the editor does while idle.

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

/// An interactive oslo on a pty, and a way to type at it and read what came back.
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
        let mut command = Command::new(common::oslo_bin());
        command
            .arg("-i")
            .env_clear()
            .env("HOME", home.path())
            .env("XDG_DATA_HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", "dumb")
            .current_dir(home.path())
            .stdin(Stdio::from(slave.try_clone().expect("clone")))
            .stdout(Stdio::from(slave.try_clone().expect("clone")))
            .stderr(Stdio::from(slave.try_clone().expect("clone")));
        // The shell needs the pty as its controlling terminal, or it is not interactive.
        //
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
        let child = command.spawn().expect("spawn oslo on a pty");
        // The parent holding a copy of the slave means the master never reaches end of file.
        drop(slave);

        let input = master.try_clone().expect("clone master");
        // A shared buffer, never joined: a child outliving the shell keeps the slave open, so the
        // reader can outlive the test.
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

    fn home(&self) -> &std::path::Path {
        self._home.path()
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().expect("transcript")).into_owned()
    }

    /// Wait for evidence rather than for a duration: under the full suite a shell takes longer to
    /// reach its first prompt than any number picked in advance.
    fn until(&self, what: impl Fn(&str) -> bool, _why: &str) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
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
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// **The whole point.** Start a job that ends on its own, then touch nothing — the notice has to
/// arrive anyway.
#[test]
fn an_idle_prompt_notices_a_job_finishing() {
    let mut shell = Shell::start();
    assert!(shell.until(|seen| !seen.trim().is_empty(), "the first prompt"));

    shell.type_line("sleep 1 &");
    assert!(
        shell.until(
            |seen| seen.contains('['),
            "the job to be announced as started"
        ),
        "the job never started:\n{}",
        shell.text()
    );

    // Nothing is typed from here. The `Done` notice can only arrive because the shell was woken.
    let noticed = shell.until(
        |seen| seen.contains("Done") || seen.contains("done"),
        "the completion notice at an idle prompt",
    );
    assert!(
        noticed,
        "an idle prompt never noticed the job finish:\n{}",
        shell.text()
    );
}

/// **A universal variable set elsewhere reaches an idle prompt, with nothing typed.**
///
/// The store is re-read before every prompt and again before every command, so typing anything at
/// all would pick it up — which is why this watches the *prompt* instead. `PS1` is single-quoted so
/// it expands each time it is drawn; if the value appears in a freshly drawn prompt without a
/// keystroke, it arrived because the editor was woken by the `inotify` watch and repainted.
#[test]
fn an_idle_prompt_sees_a_universal_set_by_another_shell() {
    let mut shell = Shell::start();
    assert!(shell.until(|seen| !seen.trim().is_empty(), "the first prompt"));

    shell.type_line("PS1='<$WOKEN># '");
    assert!(
        shell.until(|seen| seen.contains("<>#"), "the empty prompt"),
        "the prompt never took the format:\n{}",
        shell.text()
    );
    let before = shell.text().len();

    // A *second* shell writes the variable, exactly as another terminal would.
    let writer = Command::new(common::oslo_bin())
        .arg("-c")
        .arg("universal -x WOKEN=yes")
        .env("HOME", shell.home())
        .env("XDG_DATA_HOME", shell.home())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("spawn the writing shell");
    assert!(
        writer.status.success(),
        "the writer failed: {}",
        String::from_utf8_lossy(&writer.stderr)
    );

    // Nothing is typed from here.
    let arrived = shell.until(
        |seen| seen[before.min(seen.len())..].contains("<yes>#"),
        "the universal to reach the idle prompt",
    );
    assert!(
        arrived,
        "an idle prompt never saw the universal:\n{}",
        shell.text()
    );
}

/// And the shell is still usable afterwards — an interrupted `read` that is not resumed correctly
/// would eat the next keystroke or drop the line.
#[test]
fn the_prompt_still_works_after_being_woken() {
    let mut shell = Shell::start();
    assert!(shell.until(|seen| !seen.trim().is_empty(), "the first prompt"));

    shell.type_line("sleep 0.5 &");
    sleep(Duration::from_millis(1500));

    shell.type_line("echo STILL-HERE");
    assert!(
        shell.until(|seen| seen.contains("STILL-HERE"), "the shell to answer"),
        "the shell stopped reading after the wake:\n{}",
        shell.text()
    );
}
