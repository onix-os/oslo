use super::*;

#[test]
fn pasted_newlines_wait_for_an_explicit_enter() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    shell.send(b"\x1b[200~echo first\necho second\x1b[201~");
    shell.drain_for(Duration::from_millis(100));
    assert_eq!(kinds(&marks(&shell.transcript)), "AB");

    shell.send(b"\n");
    let complete = shell.wait_for_marks(6);
    assert_eq!(kinds(&complete[..6]), "ABCDAB");
}

#[test]
fn pasted_terminal_controls_are_visible_but_inert() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    shell.send(b"\x1b[200~echo \x1b]0;owned\x07 \x1b]52;c;YWJj\x1b\\ \x1b[2J\x1b[201~");
    shell.wait_for_plain_text("^[]0;owned^G");
    shell.wait_for_plain_text("^[]52;c;YWJj^[\\");
    shell.wait_for_plain_text("^[[2J");
    assert!(!shell.transcript.windows(10).any(|w| w == b"\x1b]0;owned"));
    assert!(!shell.transcript.windows(6).any(|w| w == b"\x1b[2J"));
    shell.send(b"\x03\x04");
    shell.wait_for_exit();
}

#[test]
fn grapheme_movement_and_deletion_work_through_the_real_editor() {
    let config = r#"
oslo.misc.welcome = false
oslo.prompt.left = function() return "> " end
"#;
    let mut shell = PtyShell::configured("xterm-256color", false, config);
    shell.wait_for_text("> ");

    shell.send("printf '<%s>\\n' 'e\u{301}👍🏽".as_bytes());
    shell.send(b"\x7f'\n");
    shell.wait_for_plain_text("<e\u{301}>");

    shell.send("printf '<%s>\\n' 'a👨‍👩‍👧‍👦b'".as_bytes());
    shell.send(b"\x1b[D\x1b[D\x1b[D\x1b[3~\n");
    shell.wait_for_plain_text("<ab>");
    shell.send(b"exit\n");
    shell.wait_for_exit();
}

#[test]
fn negotiated_keyboard_modes_are_balanced_on_editor_exits() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.send(b"\x1b[?1u\x1b[?62;52;c");
    shell.wait_for_occurrences(oslo::ui::term::keyboard::PUSH_DISAMBIGUATE.as_bytes(), 1);

    shell.send(b"true\n");
    shell.wait_for_occurrences(oslo::ui::term::keyboard::PUSH_DISAMBIGUATE.as_bytes(), 2);
    shell.send(b"\x1b[Z");
    shell.wait_for_occurrences(oslo::ui::term::keyboard::PUSH_DISAMBIGUATE.as_bytes(), 3);
    shell.send(b"\x03");
    shell.wait_for_occurrences(oslo::ui::term::keyboard::PUSH_DISAMBIGUATE.as_bytes(), 4);
    shell.send(b"\x04");
    shell.wait_for_exit();

    let pushes = shell
        .transcript
        .windows(oslo::ui::term::keyboard::PUSH_DISAMBIGUATE.len())
        .filter(|window| *window == oslo::ui::term::keyboard::PUSH_DISAMBIGUATE.as_bytes())
        .count();
    let pops = shell
        .transcript
        .windows(oslo::ui::term::keyboard::POP.len())
        .filter(|window| *window == oslo::ui::term::keyboard::POP.as_bytes())
        .count();
    assert_eq!(pushes, 4);
    assert_eq!(pops, pushes);
}

#[test]
fn f2_binding_works_through_the_real_editor() {
    let config = r#"
oslo.misc.welcome = false
oslo.prompt.left = function(p) return p.language .. "> " end
    oslo.keys["f2"] = "toggle-language"
"#;
    let mut shell = PtyShell::configured("xterm-256color", false, config);
    shell.send(b"\x1b[?1u\x1b[?62;52;c");
    shell.wait_for_text("sh> ");
    shell.send(b"\x1b[Q");
    shell.wait_for_text("lua> ");
    shell.send(b"\x04");
    shell.wait_for_exit();
}

#[test]
fn function_key_lua_binding_works_through_the_real_editor() {
    let config = r#"
oslo.misc.welcome = false
oslo.keys["f3"] = function(line) return "printf f3-lua" end
"#;
    let mut shell = PtyShell::configured("xterm-256color", false, config);
    shell.wait_for_marks(2);
    shell.send(b"\x1bOR");
    shell.wait_for_plain_text("printf f3-lua");
    shell.send(b"\x03\x04");
    shell.wait_for_exit();
}

#[test]
fn startup_negotiates_sync_and_notifications_before_one_barrier() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_text("\x1b]99;i=");
    let transcript = String::from_utf8_lossy(&shell.transcript);
    let id = transcript
        .split("\x1b]99;i=")
        .nth(1)
        .and_then(|rest| rest.split(':').next())
        .expect("query id")
        .to_string();
    shell.send(
        format!(
            "\x1b[?1u\x1b[?2026;1$y\x1b]99;i={id}:p=?;p=title,body:o=unfocused\x1b\\\x1b[?62;4c"
        )
        .as_bytes(),
    );
    shell.wait_for_marks(2);
    shell.send(b"status terminal\n");
    shell.wait_for_text("synchronized-output on verified");
    shell.wait_for_text("osc99-notifications on verified");
    shell.send(b"exit\n");
    shell.wait_for_exit();
}

#[test]
fn completion_replays_the_character_that_dismisses_it() {
    let mut shell = PtyShell::spawn("xterm-256color");
    shell.wait_for_marks(2);
    std::fs::write(shell._home.path().join("apple"), "").expect("apple");
    std::fs::write(shell._home.path().join("apricot"), "").expect("apricot");
    shell.send(b"cat a\t");
    shell.wait_for_plain_text("apple");
    shell.send(b"x");
    shell.wait_for_plain_text("cat ax");
    shell.send(b"\x03\x04");
    shell.wait_for_exit();
}

#[test]
fn paste_and_mouse_modes_are_balanced_on_editor_exits() {
    let mut shell =
        PtyShell::spawn_with_environment("xterm-256color", &[("OSLO_CLICK_EVENTS", "legacy")]);
    shell.wait_for_occurrences(oslo::ui::term::mouse::ENABLE, 1);
    shell.send(b"true\n");
    shell.wait_for_occurrences(oslo::ui::term::mouse::ENABLE, 2);
    shell.send(b"\x04");
    shell.wait_for_exit();

    for (enable, disable) in [
        (
            oslo::ui::term::mouse::ENABLE,
            oslo::ui::term::mouse::DISABLE,
        ),
        (
            oslo::ui::term::BRACKETED_PASTE_ENABLE,
            oslo::ui::term::BRACKETED_PASTE_DISABLE,
        ),
    ] {
        let enabled = shell
            .transcript
            .windows(enable.len())
            .filter(|w| *w == enable)
            .count();
        let disabled = shell
            .transcript
            .windows(disable.len())
            .filter(|w| *w == disable)
            .count();
        assert_eq!(enabled, 2);
        assert_eq!(disabled, enabled);
    }
}

#[test]
fn legacy_mouse_pauses_while_the_finder_owns_the_terminal() {
    let mut shell =
        PtyShell::spawn_with_environment("xterm-256color", &[("OSLO_CLICK_EVENTS", "legacy")]);
    shell.wait_for_occurrences(oslo::ui::term::mouse::ENABLE, 1);
    shell.send(b"true\n");
    shell.wait_for_occurrences(oslo::ui::term::mouse::ENABLE, 2);

    shell.send(b"\x12");
    shell.wait_for_occurrences(oslo::ui::term::mouse::DISABLE, 2);
    shell.send(b"\x1b");
    shell.wait_for_occurrences(oslo::ui::term::mouse::ENABLE, 3);

    shell.send(b"\x03");
    shell.wait_for_occurrences(oslo::ui::term::mouse::ENABLE, 4);
    shell.send(b"\x04");
    shell.wait_for_exit();

    let enabled = shell
        .transcript
        .windows(oslo::ui::term::mouse::ENABLE.len())
        .filter(|window| *window == oslo::ui::term::mouse::ENABLE)
        .count();
    let disabled = shell
        .transcript
        .windows(oslo::ui::term::mouse::DISABLE.len())
        .filter(|window| *window == oslo::ui::term::mouse::DISABLE)
        .count();
    assert_eq!(enabled, disabled);
}

#[test]
fn semantic_clicks_do_not_enable_global_mouse_reporting() {
    let mut shell =
        PtyShell::spawn_with_environment("xterm-256color", &[("OSLO_CLICK_EVENTS", "1")]);
    shell.wait_for_text("click_events=2");
    shell.drain_for(Duration::from_millis(50));
    assert!(
        !shell
            .transcript
            .windows(oslo::ui::term::mouse::ENABLE.len())
            .any(|w| { w == oslo::ui::term::mouse::ENABLE })
    );
    shell.send(b"\x1b[<0;1;1M");
    shell.drain_for(Duration::from_millis(50));
    assert!(
        !shell.transcript.windows(5).any(|w| w == b"\x1b[?6n"),
        "{}",
        visible(&shell.transcript)
    );
    shell.send(b"\x04");
    shell.wait_for_exit();
}

#[test]
fn vscode_selects_one_rich_lifecycle() {
    let mut shell = PtyShell::spawn_with_options("xterm-256color", false, Some("vscode"));
    shell.wait_for_text("\x1b]633;B\x1b\\");
    shell.send(b"true\n");
    shell.wait_for_text("\x1b]633;D;0\x1b\\");
    shell.wait_for_text("\x1b]633;B\x1b\\");
    shell.drain_for(Duration::from_millis(50));

    assert_eq!(vscode_kinds(&shell.transcript), "ABECDAB");
    assert!(marks(&shell.transcript).is_empty());
}

#[test]
fn dumb_and_noninteractive_sessions_emit_no_semantic_marks() {
    let mut dumb = PtyShell::spawn("dumb");
    dumb.wait_for_text("Type 'exit'");
    dumb.send(b"exit\n");
    dumb.wait_for_exit();
    assert!(
        marks(&dumb.transcript).is_empty(),
        "{}",
        visible(&dumb.transcript)
    );

    let output = Command::new(common::oslo_bin())
        .args(["-c", "true"])
        .env("TERM", "xterm-256color")
        .output()
        .expect("run noninteractive oslo");
    let mut transcript = output.stdout;
    transcript.extend(output.stderr);
    assert!(marks(&transcript).is_empty(), "{}", visible(&transcript));
}

#[test]
fn noninteractive_terminal_status_is_inert() {
    let output = Command::new(common::oslo_bin())
        .args(["-c", "status terminal"])
        .env("TERM", "xterm-256color")
        .env("OSLO_SYNC_OUTPUT", "1")
        .env("OSLO_CLICK_EVENTS", "1")
        .output()
        .expect("run terminal diagnostics");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("semantic-protocol none disabled\n"));
    let mut transcript = output.stdout;
    transcript.extend(output.stderr);
    assert!(!transcript.contains(&0x1b), "{}", visible(&transcript));
}
