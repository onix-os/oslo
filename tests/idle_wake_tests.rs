//! Does an *idle* prompt notice a background job finishing?
//!
//! Before this, no: the editor is blocked in `read` on the terminal, a child exiting is not a
//! keystroke, and the shell learned about it at the next command boundary — which for somebody
//! sitting at a prompt means "when you next press Enter". A job that ended ten minutes ago was
//! announced when you ran the next thing.
//!
//! The editor's wait now watches a set of descriptors beside the terminal, and everything that
//! wants its attention arrives on one of them: a pipe the `SIGCHLD` handler writes a byte to, an
//! `inotify` watch on the universal store, the queue a finished `oslo.spawn` appends to. A timer
//! is the same wait with a deadline instead of none.
//!
//! Every test here types nothing after the setup. That is the whole point — the shell has to
//! notice while nobody is touching it — so anything that arrives only at a command boundary fails
//! them, which is what they are for.
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
        Shell::with_config("")
    }

    /// The same, with a `config.lua` in place before the first prompt — which is how anything that
    /// has to be registered *without typing* gets registered.
    fn with_config(config: &str) -> Shell {
        let pty = openpty(None, None).expect("open pty");
        let master = owned(pty.master);
        let slave = owned(pty.slave);
        let home = tempfile::tempdir().expect("temporary home");
        if !config.is_empty() {
            let dir = home.path().join("config/oslo");
            std::fs::create_dir_all(&dir).expect("config directory");
            std::fs::write(dir.join("config.lua"), config).expect("config");
        }
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

/// **And a handler is told, with `source` saying it came from somewhere else.**
///
/// The field is the whole reason `on-variable-change` exists — a status line usually wants to act
/// on another terminal's change and not on its own — and this is the only place it can be proved,
/// because a script never re-reads the store.
#[test]
fn a_universal_from_another_shell_announces_itself_as_remote() {
    let shell = Shell::with_config(
        r#"oslo.on["on-variable-change"](function(e)
             print("CHANGED " .. e.name .. " " .. e.action .. " " .. e.scope .. " " .. e.source)
           end)"#,
    );
    assert!(shell.until(|seen| !seen.trim().is_empty(), "the first prompt"));

    let writer = Command::new(common::oslo_bin())
        .arg("-c")
        .arg("universal MOOD=calm")
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

    // Nothing is typed from here either.
    assert!(
        shell.until(
            |seen| seen.contains("CHANGED MOOD set universal remote"),
            "the announcement",
        ),
        "no handler was told about the remote change:\n{}",
        shell.text()
    );

    // **And a second change to a *different* name does not re-announce this one.** The whole store
    // is re-read and re-applied every time any part of it moves, so a hook that announced what it
    // read rather than what changed would tell a handler that `MOOD` changed every time anything
    // did — which is the difference between an event and a heartbeat.
    let writer = Command::new(common::oslo_bin())
        .arg("-c")
        .arg("universal OTHER=1")
        .env("HOME", shell.home())
        .env("XDG_DATA_HOME", shell.home())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .expect("spawn the writing shell");
    assert!(writer.status.success());
    assert!(
        shell.until(
            |seen| seen.contains("CHANGED OTHER"),
            "the second announcement"
        ),
        "the second change never arrived:\n{}",
        shell.text()
    );
    assert_eq!(
        shell.text().matches("CHANGED MOOD").count(),
        1,
        "an unchanged universal was announced again:\n{}",
        shell.text()
    );
}

/// **A prompt-visible change made while you sit there reaches the screen.**
///
/// The timer is the vehicle, but the claim is about the *prompt*: a handler that changes what one
/// shows used to leave the value correct in the config and invisible on the terminal until the
/// next command. Nothing is typed here, so the new prompt can only be a redraw the shell decided
/// to do on its own — which is why `oslo.ui.redraw()` was written and then not shipped: with this
/// passing, it would have been an API that asked for something already happening.
#[test]
fn a_prompt_that_changes_while_idle_is_drawn_again() {
    let shell = Shell::with_config(
        r#"local mood = "before"
           oslo.prompt.left = function() return mood .. "> " end
           oslo.after(1200, function() mood = "after" end)"#,
    );
    assert!(
        shell.until(|seen| seen.contains("before>"), "the first prompt"),
        "the config's prompt never appeared:\n{}",
        shell.text()
    );

    // Nothing is typed from here: a changed prompt can only be one the shell chose to redraw.
    assert!(
        shell.until(|seen| seen.contains("after>"), "the redraw"),
        "the prompt was never drawn again:\n{}",
        shell.text()
    );
}

/// **A timer fires while you sit there, with nothing typed at all.**
///
/// A timer used to come due only at a command boundary: set one for a few seconds, touch nothing,
/// and it fired when you next pressed Enter — the one moment its author did not mean. The idle wait
/// is now given the nearest deadline instead of "for ever".
///
/// Registered from the config so that *no* keystroke is involved, which is the whole claim.
#[test]
fn a_timer_fires_at_an_idle_prompt() {
    let shell = Shell::with_config("oslo.after(1200, function() print('TIM' .. 'ER-RANG') end)\n");
    assert!(
        shell.until(|seen| seen.contains("TIMER-RANG"), "the timer to fire"),
        "a timer never fired at an idle prompt:\n{}",
        shell.text()
    );
}

/// **A background `oslo.spawn` delivers its callback while you sit there.**
///
/// The worker finishes on a thread and appends its result; a nudge down a self-pipe is what brings
/// the idle editor to look. Without it the callback waited for the next command.
#[test]
fn a_spawn_callback_arrives_at_an_idle_prompt() {
    let shell = Shell::with_config(
        "oslo.spawn{'sleep', '1', on_exit = function() print('SPAWN' .. '-BACK') end}\n",
    );
    assert!(
        shell.until(|seen| seen.contains("SPAWN-BACK"), "the callback"),
        "a spawn callback never arrived at an idle prompt:\n{}",
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
