//! `oslo.history` against a store with something in it.
//!
//! **A real interactive shell on a pty, because that is the only kind that has a store.** The
//! tracker is installed by the REPL and by nothing else — `tracking::Tracker::start` says so — so a
//! script, an `oslo -c` and a subshell all correctly see an empty history. Testing this anywhere
//! but a pty would be testing the empty case and calling it the full one.
//!
//! The queries run from a builtin registered in `init.lua`, so what is exercised is the same path a
//! configuration would use.

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

/// An interactive oslo on a pty, with a configuration in place before the first prompt.
struct Shell {
    input: fs::File,
    seen: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    _home: tempfile::TempDir,
    child: std::process::Child,
}

impl Shell {
    fn with_config(config: &str) -> Shell {
        let pty = openpty(None, None).expect("open pty");
        let master = owned(pty.master);
        let slave = owned(pty.slave);
        let home = tempfile::tempdir().expect("temporary home");
        let dir = home.path().join("config/oslo");
        fs::create_dir_all(&dir).expect("config directory");
        fs::write(dir.join("init.lua"), config).expect("config");

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

    /// The transcript with the terminal's own escapes taken out.
    ///
    /// **A pty carries the line editor's redraws as well as the shell's output**, so a printed line
    /// arrives with cursor moves and colour changes wrapped round it — including *before* it on the
    /// same line. Matching on a prefix therefore does not work, and neither does reading a "line"
    /// without this.
    fn plain(&self) -> String {
        let raw = self.text();
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            match chars.peek() {
                // `ESC [ … <letter>` and `ESC ] … BEL`, which is every shape the editor emits.
                Some('[') => {
                    for c in chars.by_ref() {
                        if c.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                Some(']') => {
                    for c in chars.by_ref() {
                        if c == '\x07' || c == '\\' {
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
        }
        out
    }

    /// Wait for evidence rather than a duration — a debug build under the full suite is slow to
    /// reach its first prompt, and waiting costs nothing when the evidence arrives early.
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
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A configuration registering a builtin per question, so each can be asked by typing its name.
const ASKING: &str = r#"
oslo.misc.welcome = false

oslo.register_builtin{ name = "mine", run = function()
  for _, c in ipairs(oslo.history.commands{ limit = 200 }) do
    if c.line:find("marker", 1, true) then
      print("ROW " .. c.line .. " runs=" .. c.runs .. " worked=" .. tostring(c.worked)
            .. " mode=" .. c.mode .. " dir=" .. tostring(c.dir ~= "")
            .. " when=" .. tostring(c.last_at > 0))
    end
  end
  print("ASKED")
  return 0
end }

oslo.register_builtin{ name = "drop", run = function(argv)
  print("GONE " .. oslo.history.forget(argv[2]))
  return 0
end }
"#;

/// **What was run comes back, with the facts the tracker kept.** A flat history file could answer
/// the line and nothing else; every other field here is why this is worth an API.
#[test]
fn a_command_that_ran_is_in_the_history_with_its_facts() {
    let mut shell = Shell::with_config(ASKING);
    assert!(shell.until(|seen| seen.contains('$') || seen.contains('>')));

    shell.type_line("echo marker-one");
    shell.type_line("echo marker-one");
    shell.type_line("mine");

    assert!(
        shell.until(|seen| seen.contains("ASKED")),
        "the query never ran: {}",
        shell.text()
    );
    let seen = shell.plain();
    let row = seen
        .lines()
        .find(|line| line.contains("ROW echo marker-one"))
        .unwrap_or_else(|| panic!("no row for the command that ran:\n{seen}"));

    // Run twice, folded into one row with a count of two.
    assert!(row.contains("runs=2"), "{row}");
    assert!(row.contains("worked=true"), "{row}");
    assert!(row.contains("mode=sh"), "{row}");
    // The directory and the timestamp are the fields a history *file* cannot carry.
    assert!(row.contains("dir=true"), "{row}");
    assert!(row.contains("when=true"), "{row}");
}

/// **A command that failed is still listed, and says so.** You may be going back to fix it — see
/// `Command::worked`.
#[test]
fn a_command_that_failed_is_listed_and_says_it_failed() {
    let mut shell = Shell::with_config(ASKING);
    assert!(shell.until(|seen| seen.contains('$') || seen.contains('>')));

    shell.type_line("sh -c 'echo marker-bad; exit 3'");
    shell.type_line("mine");

    assert!(
        shell.until(|seen| seen.contains("ASKED")),
        "the query never ran: {}",
        shell.text()
    );
    let seen = shell.plain();
    let row = seen
        .lines()
        .find(|line| line.contains("ROW") && line.contains("marker-bad"))
        .unwrap_or_else(|| panic!("no row for the failing command:\n{seen}"));
    assert!(row.contains("worked=false"), "{row}");
}

/// **Forgetting takes it out for good**, which is the whole point of the call: a line removed from
/// the aggregate but left in the log comes back on the next start.
#[test]
fn a_forgotten_command_does_not_come_back() {
    let mut shell = Shell::with_config(ASKING);
    assert!(shell.until(|seen| seen.contains('$') || seen.contains('>')));

    shell.type_line("echo marker-secret");
    shell.type_line("mine");
    assert!(
        shell.until(|seen| seen.contains("ROW echo marker-secret")),
        "the command was never recorded: {}",
        shell.text()
    );

    shell.type_line("drop 'echo marker-secret'");
    assert!(
        shell.until(|seen| seen.contains("GONE ")),
        "forget never ran: {}",
        shell.text()
    );
    let before = shell.plain();
    let removed: usize = before
        .lines()
        .find_map(|line| line.split("GONE ").nth(1))
        .and_then(|n| n.split_whitespace().next()?.parse().ok())
        .unwrap_or_else(|| panic!("no count in:\n{before}"));
    assert!(removed > 0, "nothing was forgotten:\n{before}");

    // Ask again; the line must not be in the *new* answer.
    shell.type_line("mine");
    // Two `mine` runs, not three: the `drop` between them prints `GONE`, not `ASKED`.
    assert!(
        shell.until(|seen| seen.matches("ASKED").count() >= 2),
        "the second query never ran"
    );
    let after = shell.plain();
    let after = &after[after.rfind("GONE ").unwrap_or(0)..];
    assert!(
        !after.contains("ROW echo marker-secret"),
        "the forgotten command came back:\n{after}"
    );
}
