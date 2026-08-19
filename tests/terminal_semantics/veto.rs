//! A `pre-cmd` handler that declines to have a line written down.
//!
//! **Every sink gets its own assertion, and that is the point of this file.** The failure mode of a
//! veto is silent: the shell runs the command, prints nothing unusual, and writes the line into one
//! place nobody checked. A test that looked only at `$HISTFILE` would have passed for a version
//! that still fed the tracking store, the terminal and the desktop.
//!
//! The marker strings stand in for a credential; none of them is one.

use super::*;

/// A config that hides any line mentioning `HIDEME`, and nothing else.
const HIDING: &str = r#"
oslo.misc.welcome = false
oslo.prompt.left = function() return "> " end
oslo.on.pre_cmd(function(c)
  if c.text:match("HIDEME") then
    return { record = false }
  end
end)
"#;

/// Run one public and one hidden line, and answer the shell so its files can be read.
///
/// **`$HISTFILE` is set on purpose.** There is no default history file any more, and a veto test run
/// against a shell with one sink switched off is a weaker test — the point of this file is that
/// *every* sink is checked, so every sink has to be turned on.
fn after_both() -> PtyShell {
    let mut shell = PtyShell::spawn_with_config_and_env(
        "xterm-256color",
        true,
        None,
        Some(HIDING),
        &[("HISTFILE", ".oslo_history")],
    );
    shell.wait_for_text("> ");
    shell.send(b"echo SHOWME\n");
    shell.wait_for_plain_text("SHOWME");
    shell.send(b"echo HIDEME\n");
    shell.wait_for_plain_text("HIDEME");
    shell.send(b"exit\n");
    shell.wait_for_exit();
    shell
}

/// What the shell wrote into its home, as one string per file that exists.
fn written(shell: &PtyShell) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut stack = vec![shell.home.path().to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            // The config is the one file that legitimately contains the marker: it is the rule
            // that matches it. Everything else the shell wrote is fair game.
            if path.ends_with("init.lua") {
                continue;
            }
            // Lossily, because the tracking store is a binary file — a marker written into it is
            // still findable as bytes, which is exactly the question being asked.
            if let Ok(bytes) = std::fs::read(&path) {
                found.push((
                    path.display().to_string(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                ));
            }
        }
    }
    found
}

/// **The whole contract, file by file.** The public line has to be somewhere, or the test is
/// passing because nothing was recorded at all.
#[test]
fn a_vetoed_line_reaches_no_file_the_shell_writes() {
    let shell = after_both();
    let files = written(&shell);

    let public: Vec<&String> = files
        .iter()
        .filter(|(_, body)| body.contains("SHOWME"))
        .map(|(path, _)| path)
        .collect();
    assert!(
        !public.is_empty(),
        "the public line was not recorded anywhere, so this test proves nothing"
    );

    for (path, body) in &files {
        assert!(
            !body.contains("HIDEME"),
            "the hidden line reached {path}\n{}",
            body.chars().take(400).collect::<String>()
        );
    }
}

/// The terminal is told what is running twice over: a semantic mark carrying the whole line, and a
/// window title carrying its first word.
///
/// **The mark is the one that matters.** `marks::output_start` publishes the command line itself to
/// whatever is listening — a multiplexer, an IDE, a terminal keeping its own command history. The
/// title only ever holds the first word (`title_for_command`), so an argument could not leak there
/// even before this; hiding it as well is for the case where the *name* is the thing worth hiding.
#[test]
fn the_terminal_is_not_told_what_the_hidden_command_was() {
    let shell = after_both();
    let transcript = String::from_utf8_lossy(&shell.transcript);

    // The mark carries the line URL-encoded: `cmdline_url=echo%20SHOWME`.
    assert!(
        !transcript.contains("cmdline_url=echo%20HIDEME"),
        "the hidden command was published in a semantic mark"
    );
    // The public one is published, so the absence above is the veto at work rather than a shell
    // that stopped publishing anything.
    assert!(
        transcript.contains("cmdline_url=echo%20SHOWME"),
        "the public command was not published, so this test proves nothing"
    );
    assert!(
        transcript.contains("\u{1b}]0;private command"),
        "a hidden command should still title the window, saying only that"
    );
    assert!(
        transcript.contains("\u{1b}]0;echo — "),
        "the public command should still title the window with its name"
    );
}

/// `set -x` prints every expanded argument from inside execution, where the loop cannot reach.
#[test]
fn xtrace_does_not_print_a_hidden_commands_arguments() {
    let mut shell = PtyShell::configured("xterm-256color", true, HIDING);
    shell.wait_for_text("> ");
    shell.send(b"set -x\n");
    shell.send(b"echo SHOWME\n");
    shell.wait_for_plain_text("+ echo SHOWME");
    shell.send(b"echo HIDEME\n");
    shell.wait_for_plain_text("HIDEME");
    shell.send(b"set +x\n");
    shell.send(b"exit\n");
    shell.wait_for_exit();

    let transcript = visible(&shell.transcript);
    assert!(
        !transcript.contains("+ echo HIDEME"),
        "set -x traced the hidden command:\n{transcript}"
    );
}
