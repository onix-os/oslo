//! Terminal semantic marks observed through the real interactive binary.
mod common;

use std::time::Duration;

#[path = "terminal_semantics/veto.rs"]
mod veto;

#[path = "terminal_semantics/pty.rs"]
mod pty;

use pty::{PtyShell, kinds, marks, visible, vscode_kinds};
// `terminal_input` reads this through `use super::*`.
use std::process::Command;

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
    // **`end` does not run it; an empty line does.** A Lua block that has asked for more keeps
    // asking until a blank line, which is Python's rule and there for Python's reason: after
    // `local function f()` the parser is satisfied again at `end`, so running the moment it parses
    // would mean no line after `end` could ever be typed. So this is one more continuation prompt,
    // not the command — see `docs/features/two-languages-one-prompt.md`.
    shell.send(b"end end\n");
    shell.wait_for_marks(8);
    shell.send(b"\n");
    let complete = shell.wait_for_marks(12);
    assert_eq!(kinds(&complete[..12]), "ABABABABCDAB");
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

/// **A Ctrl-C must escape `rm`'s prompt**, and this took six presses and a walk away from the desk.
///
/// ```text
/// oslo: rm: remove write-protected regular file '…/objects/a6/b653…'? ^C^C^C^C^C^C
/// ```
///
/// `rm` is a *builtin*: it runs in the shell process, so there is no child for the terminal driver
/// to kill and the shell itself has to arrange the ending. SIGINT is installed without `SA_RESTART`
/// exactly so the blocking read fails with `EINTR` — but the prompt used `BufRead::read_line`,
/// which is `read_until`, which treats `Interrupted` as "try again". The signal arrived, the flag
/// was set, the read went straight back to waiting, and nothing looked at the flag until an answer
/// came that never would.
///
/// Driven through a pty because that is the only place the keystroke is real: the terminal driver
/// turns `^C` into the signal, and a test writing the byte down a pipe would prove nothing.
#[test]
fn a_ctrl_c_escapes_the_rm_prompt() {
    let mut shell = PtyShell::configured("xterm-256color", false, "oslo.misc.welcome = false\n");
    shell.wait_for_marks(2);

    // A file the user cannot write to, which is what makes `rm` ask without `-i`.
    let tree = shell.home.path().join("tree");
    std::fs::create_dir_all(&tree).expect("tree");
    let guarded = tree.join("guarded");
    std::fs::write(&guarded, b"x").expect("file");
    let mut mode = std::fs::metadata(&guarded).expect("stat").permissions();
    mode.set_readonly(true);
    std::fs::set_permissions(&guarded, mode).expect("chmod");

    shell.send(b"rm -r tree\n");
    shell.wait_for_text("write-protected");

    // **One press**, which is the whole point — the old code survived six.
    shell.send(&[0x03]);

    // The prompt comes back rather than the question being asked again for ever.
    let after = shell.wait_for_marks(6);
    assert_eq!(
        kinds(&after[..6]),
        "ABCDAB",
        "{:?}",
        visible(&shell.transcript)
    );

    // And nothing was removed on the way out.
    assert!(
        guarded.exists(),
        "the interrupted rm removed the file anyway"
    );
}

/// **Stdout comes back when a structured pipeline gives up part-way.**
///
/// The tools half of a pipeline points this process's stdout at a scratch file and puts it back in
/// `finish`. Any error between the two — an expansion failure in a later stage, either fallback
/// arm — used to skip `finish` entirely, leaving stdout dup2'd onto a *deleted* temporary file for
/// the rest of the session. The prompt still drew, so the shell looked alive; nothing said after it
/// ever reached the terminal again, builtin or external alike.
#[test]
fn a_pipeline_that_fails_part_way_gives_the_terminal_back() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    // `first` is a structured tool, so the tools half takes stdout; the unset-parameter error then
    // aborts the line before the half that hands it back.
    shell.send(b"ls | first ${nope?boom} | cat\n");
    // **Asserted on output the input cannot contain.** `$?` is written literally in what is typed
    // and expands only in what is printed, so `ttycheck=0` can only be the answer — where matching
    // the typed word itself would pass against the terminal echo whether or not stdout came back.
    shell.send(b"test -t 1; echo \"ttycheck=$?\"\n");
    shell.wait_for_plain_text("ttycheck=0");
}

/// **A job hook may ask the shell about its jobs.**
///
/// `on-job-finish` fired from inside the job table's lock, which is a plain non-reentrant `Mutex` —
/// so a handler calling `oslo.job.list()` waited for a lock its own caller held, and the shell
/// wedged the first time any background command finished. The rule was already written down for
/// the sibling `announce`; these two hooks were simply on the wrong side of it.
///
/// Interactive because that is the only place the notice is drawn: `announce_changes` returns
/// early when the shell is not interactive, so a non-interactive test cannot reach the deadlock.
#[test]
fn a_job_finish_handler_may_ask_about_jobs() {
    let mut shell = PtyShell::spawn_with_config(
        "xterm-256color",
        false,
        None,
        Some("oslo.on.job_finish(function() local _ = #oslo.job.list() end)\n"),
    );
    shell.wait_for_marks(2);
    shell.send(b"sleep 0.1 &\n");
    shell.send(b"sleep 0.4\n");
    // Asserted on output the typed line cannot contain — the arithmetic is literal in the input
    // and only the answer appears in the output. A wedged shell never gets here.
    shell.send(b"echo \"alive=$((6*7))\"\n");
    shell.wait_for_plain_text("alive=42");
}

/// **The slow-command notification is reaped.**
///
/// `Child` does not reap when it is dropped, SIGCHLD is caught rather than ignored so the kernel
/// does not either, and the reaper asks only about pids the job table names — so every command over
/// `oslo.notify.after` left an `[sh] <defunct>` behind, one per slow command, holding a pid slot
/// against `RLIMIT_NPROC` until something typed `wait`.
///
/// Driven through a pty because the notice is a between-commands side effect of an interactive
/// shell, which is the only place it fires.
#[test]
fn a_slow_command_notice_leaves_no_zombie() {
    let config = r#"
oslo.misc.welcome = false
oslo.notify.after = 1
oslo.notify.command = "true"
"#;
    let mut shell = PtyShell::configured("xterm-256color", true, config);
    shell.wait_for_marks(2);

    // Three commands over the threshold, so three notices are spawned.
    for _ in 0..3 {
        shell.send(b"sleep 1.2\n");
    }
    shell.wait_for_marks(14);

    // Ask the shell itself what it has left behind — `ps` sees its own children. The marker is
    // *built* by the command rather than typed, so waiting for it cannot match the echo of the
    // line: what was typed contains `$(`, and only the answer contains `ZOMBIES=0`.
    shell.send(b"echo \"ZOMBIES=$(ps --ppid $$ -o stat= 2>/dev/null | grep -c '^Z')\"\n");
    shell.wait_for_plain_text("ZOMBIES=0");
}
