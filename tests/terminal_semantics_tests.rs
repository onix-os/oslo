//! Terminal semantic marks observed through the real interactive binary.
mod common;

use nix::pty::openpty;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(5);

#[path = "terminal_semantics/veto.rs"]
mod veto;

#[derive(Debug)]
struct Mark {
    kind: u8,
    body: String,
}

impl Mark {
    fn status(&self) -> Option<i32> {
        (self.kind == b'D')
            .then(|| self.body.split(';').nth(1)?.parse().ok())
            .flatten()
    }

    fn aid(&self) -> Option<&str> {
        self.body
            .split(';')
            .find_map(|field| field.strip_prefix("aid="))
    }
}

struct PtyShell {
    child: Child,
    input: File,
    output: Receiver<Vec<u8>>,
    transcript: Vec<u8>,
    home: tempfile::TempDir,
}

impl PtyShell {
    fn spawn(term: &str) -> Self {
        Self::spawn_with_options(term, false, None)
    }

    fn spawn_with_extensions(term: &str, semantic_extensions: bool) -> Self {
        Self::spawn_with_options(term, semantic_extensions, None)
    }

    fn spawn_with_options(
        term: &str,
        semantic_extensions: bool,
        term_program: Option<&str>,
    ) -> Self {
        Self::spawn_with_config(term, semantic_extensions, term_program, None)
    }

    fn configured(term: &str, semantic_extensions: bool, config: &str) -> Self {
        Self::spawn_with_config(term, semantic_extensions, None, Some(config))
    }

    fn spawn_with_config(
        term: &str,
        semantic_extensions: bool,
        term_program: Option<&str>,
        config: Option<&str>,
    ) -> Self {
        Self::spawn_with_config_and_env(term, semantic_extensions, term_program, config, &[])
    }

    fn spawn_with_environment(term: &str, environment: &[(&str, &str)]) -> Self {
        Self::spawn_with_config_and_env(term, false, None, None, environment)
    }
    fn spawn_with_config_and_env(
        term: &str,
        semantic_extensions: bool,
        term_program: Option<&str>,
        config: Option<&str>,
        environment: &[(&str, &str)],
    ) -> Self {
        let pty = openpty(None, None).expect("open pty");
        let master: File = owned_file(pty.master);
        let slave: File = owned_file(pty.slave);
        let stdin = slave.try_clone().expect("clone pty slave");
        let stdout = slave.try_clone().expect("clone pty slave");
        let home = tempfile::tempdir().expect("temporary home");
        if let Some(config) = config {
            let directory = home.path().join(".config/oslo");
            std::fs::create_dir_all(&directory).expect("create config directory");
            std::fs::write(directory.join("init.lua"), config).expect("write config");
        }
        let mut command = Command::new(common::oslo_bin());
        command
            .arg("-i")
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .env("TERM", term)
            .env("COLORFGBG", "15;0")
            .current_dir(home.path())
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(slave));
        if semantic_extensions {
            command.env("OSLO_TERMINAL_EXTENSIONS", "kitty");
        }
        if let Some(term_program) = term_program {
            command.env("TERM_PROGRAM", term_program);
        }
        command.envs(environment.iter().copied());
        // SAFETY: this runs after fork and only calls async-signal-safe system interfaces.
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
        let child = command.spawn().expect("spawn oslo on pty");
        let input = master.try_clone().expect("clone pty master");
        let (send, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut master = master;
            loop {
                let mut bytes = vec![0; 4096];
                match master.read(&mut bytes) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        bytes.truncate(read);
                        if send.send(bytes).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Self {
            child,
            input,
            output,
            transcript: Vec::new(),
            home,
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).expect("write pty input");
        self.input.flush().expect("flush pty input");
    }

    fn wait_for_marks(&mut self, count: usize) -> Vec<Mark> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let marks = marks(&self.transcript);
            if marks.len() >= count {
                return marks;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out: {:?}",
                visible(&self.transcript)
            );
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.transcript.extend(bytes),
                Err(error) => panic!("pty ended before {count} marks: {error:?}"),
            }
        }
    }

    fn wait_for_text(&mut self, text: &str) {
        let deadline = Instant::now() + TIMEOUT;
        while !String::from_utf8_lossy(&self.transcript).contains(text) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out: {:?}",
                visible(&self.transcript)
            );
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.transcript.extend(bytes),
                Err(error) => panic!(
                    "pty ended before {text:?}: {error:?}: {}",
                    visible(&self.transcript)
                ),
            }
        }
    }

    /// Wait until `text` has appeared `times` times, not merely once.
    ///
    /// **[`Self::wait_for_text`] scans the whole transcript**, so waiting for something the shell
    /// emits once per prompt returns instantly the second time you ask — the *first* prompt's copy
    /// is still there. A test that then reads the transcript is really relying on a drain to
    /// happen to catch the rest, which is a race: `vscode_selects_one_rich_lifecycle` passed on a
    /// quiet machine for months and failed on a loaded CI runner, one prompt's marks short.
    fn wait_for_text_count(&mut self, text: &str, times: usize) {
        let deadline = Instant::now() + TIMEOUT;
        while String::from_utf8_lossy(&self.transcript)
            .matches(text)
            .count()
            < times
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for {times} of {text:?}: {:?}",
                visible(&self.transcript)
            );
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.transcript.extend(bytes),
                Err(error) => panic!(
                    "pty ended before {times} of {text:?}: {error:?}: {}",
                    visible(&self.transcript)
                ),
            }
        }
    }

    fn wait_for_plain_text(&mut self, text: &str) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let rendered = String::from_utf8_lossy(&self.transcript);
            let plain = oslo::ui::dropdown::width::without_escapes(&rendered);
            if plain.contains(text) {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out before {text:?}: {}",
                visible(&self.transcript)
            );
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.transcript.extend(bytes),
                Err(error) => panic!(
                    "pty ended before {text:?}: {error:?}: {}",
                    visible(&self.transcript)
                ),
            }
        }
    }

    fn wait_for_occurrences(&mut self, bytes: &[u8], count: usize) {
        let deadline = Instant::now() + TIMEOUT;
        while self
            .transcript
            .windows(bytes.len())
            .filter(|window| *window == bytes)
            .count()
            < count
        {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out: {:?}",
                visible(&self.transcript)
            );
            match self.output.recv_timeout(remaining) {
                Ok(output) => self.transcript.extend(output),
                Err(error) => panic!("pty ended before {count} matches: {error:?}"),
            }
        }
    }

    fn wait_for_exit(&mut self) {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            while let Ok(bytes) = self.output.try_recv() {
                self.transcript.extend(bytes);
            }
            if self.child.try_wait().expect("wait for oslo").is_some() {
                while let Ok(bytes) = self.output.recv_timeout(Duration::from_millis(20)) {
                    self.transcript.extend(bytes);
                }
                return;
            }
            assert!(Instant::now() < deadline, "oslo did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn drain_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.output.recv_timeout(remaining) {
                Ok(bytes) => self.transcript.extend(bytes),
                Err(_) => break,
            }
        }
    }
}

impl Drop for PtyShell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn owned_file(fd: OwnedFd) -> File {
    fd.into()
}

fn marks(bytes: &[u8]) -> Vec<Mark> {
    let mut found = Vec::new();
    let mut at = 0;
    while at + 6 <= bytes.len() {
        if bytes[at..].starts_with(b"\x1b]133;")
            && let Some(end) = bytes[at + 6..]
                .windows(2)
                .position(|window| window == b"\x1b\\")
        {
            let body = &bytes[at + 6..at + 6 + end];
            if let Some(kind) = body.first().copied() {
                found.push(Mark {
                    kind,
                    body: String::from_utf8_lossy(body).into_owned(),
                });
            }
            at += 6 + end + 2;
        } else {
            at += 1;
        }
    }
    found
}

fn visible(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).escape_debug().to_string()
}

fn kinds(marks: &[Mark]) -> String {
    marks.iter().map(|mark| mark.kind as char).collect()
}

fn vscode_kinds(bytes: &[u8]) -> String {
    let mut kinds = String::new();
    let mut at = 0;
    while at + 6 <= bytes.len() {
        if bytes[at..].starts_with(b"\x1b]633;")
            && let Some(end) = bytes[at + 6..]
                .windows(2)
                .position(|window| window == b"\x1b\\")
        {
            if let Some(kind @ (b'A' | b'B' | b'C' | b'D' | b'E')) = bytes.get(at + 6) {
                kinds.push(*kind as char);
            }
            at += 6 + end + 2;
        } else {
            at += 1;
        }
    }
    kinds
}

#[test]
fn successful_and_failed_commands_have_balanced_stable_marks() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    shell.send(b"true\nfalse\n");
    let marks = shell.wait_for_marks(10);

    assert_eq!(kinds(&marks[..10]), "ABCDABCDAB");
    assert_eq!(marks[3].status(), Some(0));
    assert_eq!(marks[7].status(), Some(1));
    let aid = marks[0].aid().expect("session aid");
    assert!(marks[..10].iter().all(|mark| mark.aid() == Some(aid)));
}

#[test]
fn blank_partial_interrupt_and_eof_close_without_command_start() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    shell.send(b"\n");
    shell.wait_for_marks(5);
    shell.send(b"partial\x03");
    shell.wait_for_marks(8);
    shell.send(b"\x03");
    shell.wait_for_marks(11);
    shell.send(b"\x04");
    shell.wait_for_exit();
    let marks = marks(&shell.transcript);

    assert_eq!(kinds(&marks), "ABDABDABDABD");
    assert!(
        marks
            .iter()
            .filter(|mark| mark.kind == b'D')
            .all(|mark| mark.status().is_none())
    );
    assert!(!marks.iter().any(|mark| mark.kind == b'C'));
}

#[test]
fn multiline_input_has_secondary_prompt_marks_in_one_interaction() {
    let mut shell = PtyShell::spawn_with_extensions("xterm-256color", true);
    shell.wait_for_marks(2);
    shell.send(b"printf '%s\\n' \"left\n");
    let continued = shell.wait_for_marks(4);
    assert_eq!(kinds(&continued[..4]), "ABAB");
    assert!(continued[2].body.contains("k=s"), "{:?}", continued[2]);
    shell.send(b"middle\n");
    let continued = shell.wait_for_marks(6);
    assert_eq!(kinds(&continued[..6]), "ABABAB");
    assert!(continued[4].body.contains("k=s"), "{:?}", continued[4]);
    shell.send(b"right\"\n");
    let complete = shell.wait_for_marks(10);

    assert_eq!(kinds(&complete[..10]), "ABABABCDAB");
    let aid = complete[0].aid().expect("session aid");
    assert!(complete[..10].iter().all(|mark| mark.aid() == Some(aid)));
}

#[test]
fn lua_continuations_and_language_redraw_keep_one_interaction() {
    let config = r#"
oslo.misc.welcome = false
oslo.env.set("OSLO_DEFAULT_MODE", "lua")
"#;
    let mut shell = PtyShell::configured("xterm-256color", true, config);
    shell.wait_for_marks(2);
    shell.send(b"if true then\n");
    shell.wait_for_marks(4);
    shell.send(b"if true then\n");
    shell.wait_for_marks(6);
    shell.send(b"end end\n");
    let complete = shell.wait_for_marks(10);
    assert_eq!(kinds(&complete[..10]), "ABABABCDAB");
}

#[test]
fn right_transient_and_vi_redraws_do_not_repeat_boundaries() {
    let config = r#"
oslo.misc.welcome = false
oslo.vi.enabled = true
oslo.prompt.left = function() return "LEFT> " end
oslo.prompt.right = function() return "RIGHT" end
oslo.prompt.transient = function() return "SHORT> " end
"#;
    let mut shell = PtyShell::configured("xterm-256color", false, config);
    shell.wait_for_marks(2);
    shell.wait_for_text("RIGHT");
    shell.send(b"\x1b[Z");
    shell.drain_for(Duration::from_millis(50));
    assert_eq!(kinds(&marks(&shell.transcript)), "AB");
    shell.send(b"\x1b[Z");
    shell.drain_for(Duration::from_millis(50));
    assert_eq!(kinds(&marks(&shell.transcript)), "AB");
    shell.send(b"\x1b");
    shell.drain_for(Duration::from_millis(50));
    assert_eq!(kinds(&marks(&shell.transcript)), "AB");

    shell.send(b"itrue\n");
    let complete = shell.wait_for_marks(6);
    assert_eq!(kinds(&complete[..6]), "ABCDAB");
    shell.wait_for_text("SHORT> ");
}

#[test]
fn post_hook_metadata_is_exact_private_and_cancel_safe() {
    let config = r#"
oslo.misc.welcome = false
oslo.on.pre_cmd(function(command)
  if command.text == "replace-me" then return "printf replaced" end
  if command.text == "cancel-me" then return false end
end)
"#;
    let mut shell = PtyShell::configured("xterm-256color", true, config);
    shell.wait_for_marks(2);
    shell.send(b"replace-me\n");
    let replaced = shell.wait_for_marks(6);
    assert!(
        replaced[2].body.contains("cmdline_url=printf%20replaced"),
        "{:?}",
        replaced[2]
    );

    shell.send(b" true\n");
    let private = shell.wait_for_marks(10);
    assert_eq!(private[6].kind, b'C');
    assert!(!private[6].body.contains("cmdline_url"), "{:?}", private[6]);

    shell.send(b"cancel-me\n");
    let cancelled = shell.wait_for_marks(13);
    assert_eq!(kinds(&cancelled[..13]), "ABCDABCDABDAB");
    assert_eq!(cancelled[10].kind, b'D');
    assert_eq!(cancelled[10].status(), None);
}

#[test]
fn disabled_marks_and_separate_processes_are_isolated() {
    let config = r#"
oslo.misc.welcome = false
oslo.feature.set("marks", false)
oslo.prompt.left = function() return "READY> " end
"#;
    let mut disabled = PtyShell::configured("xterm-256color", false, config);
    disabled.wait_for_text("READY> ");
    disabled.send(b"exit\n");
    disabled.wait_for_exit();
    assert!(marks(&disabled.transcript).is_empty());

    let mut first = PtyShell::spawn("xterm-256color");
    let first_aid = first.wait_for_marks(2)[0]
        .aid()
        .expect("first session aid")
        .to_string();
    let mut second = PtyShell::spawn("xterm-256color");
    let second_aid = second.wait_for_marks(2)[0]
        .aid()
        .expect("second session aid")
        .to_string();
    assert_ne!(first_aid, second_aid);
}

#[test]
fn iterm_nested_shells_keep_distinct_stable_session_ids() {
    // An interactive oslo started inside one asks whether that is what you meant, and this test
    // nests on purpose — so it says so in the config it starts with. See `startup::nested`.
    let mut shell = PtyShell::spawn_with_config(
        "xterm-256color",
        false,
        Some("iTerm.app"),
        Some("oslo.misc.nested_ask = false\n"),
    );
    shell.wait_for_marks(2);
    shell.send(format!("{} -i\n", common::oslo_bin().display()).as_bytes());
    let nested = shell.wait_for_marks(5);
    let outer = nested[0].aid().expect("outer aid").to_string();
    let child = nested[3].aid().expect("child aid").to_string();
    assert_ne!(outer, child);
    assert!(nested[..3].iter().all(|mark| mark.aid() == Some(&outer)));
    assert!(nested[3..5].iter().all(|mark| mark.aid() == Some(&child)));

    shell.send(b"exit 7\n");
    let finished = shell.wait_for_marks(10);
    assert!(finished[3..7].iter().all(|mark| mark.aid() == Some(&child)));
    assert_eq!(finished[6].status(), Some(7));
    assert!(
        finished[7..10]
            .iter()
            .all(|mark| mark.aid() == Some(&outer))
    );
}

/// **A shell inside a shell asks first**, and the answer that costs nothing is the one Enter gives.
///
/// On a terminal, because that is the whole condition: `$OSLO_NESTED` says there is an oslo above
/// this one, and there is a person here to ask. The test above turns this off to nest on purpose.
#[test]
fn a_nested_interactive_shell_asks_before_it_starts() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    shell.send(format!("{} -i\n", common::oslo_bin().display()).as_bytes());
    shell.wait_for_text("Start a nested shell?");

    // Enter takes the default, which is to stay where you are — so the nested shell never starts
    // and the outer one is still the shell taking input.
    shell.send(b"\r");
    shell.send(b"echo STILL-OUTER=$OSLO_NESTED\n");
    shell.wait_for_text("STILL-OUTER=0");
}

#[path = "terminal_semantics/terminal_input.rs"]
mod terminal_input;
