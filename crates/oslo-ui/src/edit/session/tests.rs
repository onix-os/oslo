//! The state machine, driven a key at a time with no terminal anywhere.
//!
//! This is the payoff of splitting `apply` from the loop: what a key does to a line can be checked
//! exhaustively, and the only thing left to get wrong on a real terminal is the drawing — which
//! `super::screen` covers separately.

use super::*;

/// Feed a sequence of keys and hand back the line.
fn run(start: &str, keys: &[Key]) -> (Session, Vec<Step>) {
    // The emacs keymap, explicitly: vi mode is the default, and these assert what a key does when
    // it is *not* a vi command.
    let mut session = Session {
        vi: None,
        ..Session::new(start, start.chars().count())
    };
    let mut assist = NoAssist;
    let steps = keys
        .iter()
        .map(|k| session.apply(*k, &mut assist))
        .collect();
    (session, steps)
}

fn typed(text: &str) -> Vec<Key> {
    text.chars().map(Key::Char).collect()
}

#[test]
fn typing_builds_a_line() {
    let (s, _) = run("", &typed("echo hi"));
    assert_eq!(s.buffer.text(), "echo hi");
    assert_eq!(s.buffer.cursor(), 7);
}

#[test]
fn editing_keys_reach_the_buffer() {
    let (s, _) = run("echo hello", &[Key::Ctrl('w')]);
    assert_eq!(s.buffer.text(), "echo ", "C-w took the last word");

    let (s, _) = run("echo hello", &[Key::Home, Key::Ctrl('k')]);
    assert_eq!(s.buffer.text(), "", "C-k from the start takes everything");

    let (s, _) = run("abc", &[Key::Left, Key::Backspace]);
    assert_eq!(s.buffer.text(), "ac");
}

/// **Ctrl-D is end of input only on an empty line.** With text it deletes forward, and getting
/// this backwards means a stray keypress closes somebody's shell.
#[test]
fn ctrl_d_is_eof_only_when_the_line_is_empty() {
    let mut empty = Session {
        vi: None,
        ..Session::new("", 0)
    };
    assert_eq!(empty.apply(Key::Delete, &mut NoAssist), Step::Eof);

    let mut has_text = Session {
        vi: None,
        ..Session::new("ls", 0)
    };
    assert_eq!(
        has_text.apply(Key::Delete, &mut NoAssist),
        Step::Continue { redraw: true }
    );
    assert_eq!(has_text.buffer.text(), "s", "it deleted forward");
}

#[test]
fn enter_accepts_and_ctrl_c_interrupts() {
    let mut s = Session {
        vi: None,
        ..Session::new("ls", 2)
    };
    assert_eq!(s.apply(Key::Accept, &mut NoAssist), Step::Accept);
    assert_eq!(s.apply(Key::Abort, &mut NoAssist), Step::Interrupted);
}

/// A key that changes nothing must not ask for a repaint — a redraw per unbound keypress is
/// visible as flicker on a slow link.
#[test]
fn a_key_that_changes_nothing_does_not_ask_for_a_redraw() {
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    for key in [Key::Ctrl('q'), Key::Alt('z'), Key::Ignored, Key::Cancel] {
        assert_eq!(
            s.apply(key, &mut NoAssist),
            Step::Continue { redraw: false },
            "{key:?} asked for a repaint"
        );
    }
    // And nor does a kill with nothing to kill.
    assert_eq!(
        s.apply(Key::Ctrl('k'), &mut NoAssist),
        Step::Continue { redraw: false }
    );
}

/// Esc alone is not "abandon the line". A shell prompt has nothing to cancel back to, and
/// Ctrl-C is the key that abandons.
///
/// **In emacs mode**, stated explicitly: with vi mode on — which is oslo's default — Esc leaves
/// insert mode, which is very much something. Building the session with `vi: None` is what makes
/// this test about the emacs keymap rather than about whichever default is current.
#[test]
fn escape_alone_does_nothing_in_emacs_mode() {
    let mut s = Session {
        vi: None,
        ..Session::new("half typed", 10)
    };
    assert_eq!(
        s.apply(Key::Cancel, &mut NoAssist),
        Step::Continue { redraw: false }
    );
    assert_eq!(s.buffer.text(), "half typed", "the line survived Esc");
}

/// An `Assist` that answers with fixed lines, to check the wiring without oslo's real machinery.
///
/// `at` counts how many steps back into history the walk has gone, so `0` means "still on the
/// line the user was typing". Modelled properly rather than as a bare index because the
/// interesting case is walking back *out* of history.
#[derive(Default)]
struct Canned {
    history: Vec<String>,
    at: usize,
    /// What was on the line when the walk started, so coming back down can restore it.
    typed: Option<String>,
    /// What the ghost suggestion would add, if anything.
    hint: Option<String>,
    /// What the line should have said, if it looks mistyped.
    repair: Option<String>,
}

impl Assist for Canned {
    /// As the real one does: no suggestion unless the cursor is at the end of the line.
    fn hint_text(&mut self, line: &str, cursor: usize) -> Option<String> {
        (cursor >= line.chars().count()).then(|| self.hint.clone())?
    }
    fn repair_text(&mut self, line: &str, cursor: usize) -> Option<String> {
        (cursor >= line.chars().count()).then(|| self.repair.clone())?
    }
    fn history_prev(&mut self, line: &str) -> Option<String> {
        let entry = self.history.get(self.at)?.clone();
        if self.at == 0 {
            self.typed = Some(line.to_string());
        }
        self.at += 1;
        Some(entry)
    }
    fn history_next(&mut self) -> Option<String> {
        match self.at {
            0 => None,
            // Stepping out of history: the line being composed comes back, not a blank.
            1 => {
                self.at = 0;
                self.typed.take()
            }
            _ => {
                self.at -= 1;
                self.history.get(self.at - 1).cloned()
            }
        }
    }
}

#[test]
fn history_walks_through_the_assist() {
    let mut a = Canned {
        history: vec!["cargo test".into(), "ls -la".into()],
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    s.apply(Key::Up, &mut a);
    assert_eq!(s.buffer.text(), "cargo test");
    assert_eq!(s.buffer.cursor(), 10, "the cursor lands at the end");
    s.apply(Key::Up, &mut a);
    assert_eq!(s.buffer.text(), "ls -la");
    s.apply(Key::Down, &mut a);
    assert_eq!(s.buffer.text(), "cargo test");
}

/// **Walking back down past the newest entry restores what you were typing.** Blanking the line
/// instead is the behaviour people lose work to, and oslo already promises not to.
#[test]
fn coming_back_out_of_history_restores_the_typed_line() {
    let mut a = Canned {
        history: vec!["cargo test".into()],
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("half-writ", 9)
    };
    s.apply(Key::Up, &mut a);
    assert_eq!(s.buffer.text(), "cargo test");
    s.apply(Key::Down, &mut a);
    assert_eq!(s.buffer.text(), "half-writ", "the composed line came back");
}

/// History that runs out leaves the line alone rather than blanking it.
#[test]
fn history_that_runs_out_changes_nothing() {
    let mut a = Canned::default();
    let mut s = Session {
        vi: None,
        ..Session::new("typed", 5)
    };
    assert_eq!(s.apply(Key::Up, &mut a), Step::Continue { redraw: false });
    assert_eq!(s.buffer.text(), "typed");
}

/// Completion is delegated to the outer loop that owns the terminal reader.
#[test]
fn completion_requests_the_shared_modal() {
    let mut a = Canned::default();
    let mut s = Session {
        vi: None,
        ..Session::new("git ch", 6)
    };
    assert_eq!(
        s.apply(Key::ToggleScope, &mut a),
        Step::OpenCompletion { backwards: false }
    );
    assert_eq!(s.buffer.text(), "git ch");
}

/// Ctrl-L asks for the screen to be cleared, which the loop does — the buffer is untouched.
#[test]
fn ctrl_l_clears_the_screen_without_touching_the_line() {
    let mut s = Session {
        vi: None,
        ..Session::new("half typed", 4)
    };
    assert_eq!(s.apply(Key::Ctrl('l'), &mut NoAssist), Step::ClearScreen);
    assert_eq!(s.buffer.text(), "half typed");
    assert_eq!(s.buffer.cursor(), 4, "and the cursor stayed put");
}

/// **Right at the end of the line accepts the ghost suggestion.** The key that reads as "yes,
/// that one" — Tab is for choosing between several, Right is for taking the one on screen.
#[test]
fn right_at_the_end_of_the_line_takes_the_suggestion() {
    let mut a = Canned {
        hint: Some(" --release".into()),
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("cargo build", 11)
    };
    assert_eq!(s.apply(Key::Right, &mut a), Step::Continue { redraw: true });
    assert_eq!(s.buffer.text(), "cargo build --release");
    assert_eq!(
        s.buffer.cursor(),
        21,
        "and the cursor follows it to the end"
    );
}

/// **In vi's normal mode Right is a motion and nothing else.** There it is `l`, and `d<Right>`
/// has to delete a character — a Right that inserted the suggestion instead would turn every
/// operator that takes it into something else. Accepting belongs to the modes where text is
/// being added.
#[test]
fn right_is_only_a_motion_in_vi_normal_mode() {
    let mut a = Canned {
        hint: Some(" -la".into()),
        ..Canned::default()
    };
    let mut s = Session::new("ls", 2);
    s.vi = Some(crate::edit::vi::Vi::default());
    s.apply(Key::Cancel, &mut a); // Esc: into normal mode, and one step left as vi does
    s.apply(Key::Right, &mut a);
    assert_eq!(s.buffer.text(), "ls", "the suggestion was not taken");

    // Back to insert, and the same key takes it.
    s.apply(Key::Char('a'), &mut a);
    s.buffer.move_end();
    s.apply(Key::Right, &mut a);
    assert_eq!(s.buffer.text(), "ls -la");
}

/// Mid-line it is still an ordinary cursor move — the suggestion is only ever drawn at the end,
/// so accepting one from the middle would insert text nobody was shown.
#[test]
fn right_in_the_middle_of_a_line_only_moves() {
    let mut a = Canned {
        hint: Some(" --release".into()),
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("cargo build", 5)
    };
    s.apply(Key::Right, &mut a);
    assert_eq!(s.buffer.text(), "cargo build", "unchanged");
    assert_eq!(s.buffer.cursor(), 6);
}

/// With nothing to suggest, Right falls through to moving — it must not become a dead key at the
/// end of a line, which is where it is pressed most.
#[test]
fn right_with_no_suggestion_still_moves() {
    let mut s = Session {
        vi: None,
        ..Session::new("ls", 1)
    };
    s.apply(Key::Right, &mut Canned::default());
    assert_eq!(s.buffer.cursor(), 2);
}

/// **The finished line is drawn without its ghost.**
///
/// The suggestion is a proposal, not text you typed, so the last frame of a line must not carry
/// it: `cat ~/` with `lis/` suggested left `cat ~/lis/` in the scrollback above the output of
/// `cat ~/`, a transcript that says a command ran which never did.
#[test]
fn the_last_frame_of_a_line_has_no_ghost() {
    let mut a = Canned {
        hint: Some("lis/".into()),
        ..Canned::default()
    };
    let s = Session {
        vi: None,
        ..Session::new("cat /home/me/", 13)
    };

    let editing = super::draw("$ ", "", &s, &mut a, true);
    assert!(
        editing.text.contains("lis/"),
        "the ghost must be drawn while editing: {:?}",
        editing.text
    );

    let finished = super::draw("$ ", "", &s, &mut a, false);
    assert!(
        !finished.text.contains("lis/"),
        "the ghost survived into the finished line: {:?}",
        finished.text
    );
    // The line itself is untouched — only the proposal goes.
    assert!(
        finished.text.contains("cat /home/me/"),
        "{:?}",
        finished.text
    );
}

#[test]
fn editor_frames_encode_untrusted_controls_but_keep_raw_text() {
    let raw = "echo \x1b]52;c;owned\x07\r\t\0\x7f";
    let s = Session {
        vi: None,
        ..Session::new(raw, raw.chars().count())
    };
    let frame = super::draw("$ ", "", &s, &mut NoAssist, true);
    assert!(
        !frame.text.contains('\x1b'),
        "raw escape reached frame: {frame:?}"
    );
    assert!(
        !frame.text.contains('\x07'),
        "raw BEL reached frame: {frame:?}"
    );
    assert!(frame.text.contains("^[]52;c;owned^G^M^I^@^?"));
    assert_eq!(s.buffer.text(), raw);
}

#[test]
fn untrusted_suggestion_controls_are_encoded_before_styling() {
    let mut assist = Canned {
        hint: Some("\x1b]0;owned\x07".into()),
        ..Canned::default()
    };
    let s = Session {
        vi: None,
        ..Session::new("echo", 4)
    };
    let frame = super::draw("$ ", "", &s, &mut assist, true);
    assert!(!frame.text.contains("\x1b]0;owned"));
    assert!(frame.text.contains("^[]0;owned^G"));
}

#[test]
fn hooks_receive_raw_controls_instead_of_display_notation() {
    struct RawHook;
    impl Assist for RawHook {
        fn watches_keys(&mut self) -> bool {
            true
        }

        fn key_hook(&mut self, _key: Key, line: &str, cursor: usize) -> Option<KeyHook> {
            assert_eq!(line, "echo \x1b]0;owned\x07");
            assert_eq!(cursor, line.chars().count());
            Some(KeyHook::Swallow)
        }
    }

    let raw = "echo \x1b]0;owned\x07";
    let mut session = Session {
        vi: None,
        ..Session::new(raw, raw.chars().count())
    };
    assert_eq!(
        session.apply(Key::Function(1), &mut RawHook),
        Step::Continue { redraw: false }
    );
    assert_eq!(session.buffer.text(), raw);
}

/// A Lua binding may run the line, not only fill it in.
///
/// zsh spells this `bindkey -s '^[a' ' _a\n'`, and the trailing newline is the whole point: the
/// key runs something. A handler that could only set the text left the line sitting there.
#[test]
fn a_lua_binding_can_submit_the_line() {
    struct Bind {
        submit: bool,
    }
    impl Assist for Bind {
        fn binding(&mut self, key: Key) -> Option<Bound> {
            (key == Key::Alt('a')).then(|| Bound::Lua("alt-a".into()))
        }
        fn lua_key(&mut self, _n: &str, _l: &str, _c: usize) -> Option<(String, usize, bool)> {
            Some((" _a".to_string(), 3, self.submit))
        }
    }

    let mut s = Session {
        vi: None,
        ..Session::new("half typed", 10)
    };
    assert_eq!(
        s.apply(Key::Alt('a'), &mut Bind { submit: true }),
        Step::Accept,
        "submit = true runs the line"
    );
    assert_eq!(
        s.buffer.text(),
        " _a",
        "and it is the handler's line that runs"
    );

    let mut s = Session {
        vi: None,
        ..Session::new("half typed", 10)
    };
    assert_eq!(
        s.apply(Key::Alt('a'), &mut Bind { submit: false }),
        Step::Continue { redraw: true },
        "without it the line is only filled in"
    );
    assert_eq!(s.buffer.text(), " _a");
}

/// The `key` hook sees a key before any binding, before vi, and before an ordinary character is
/// inserted — and each of its three answers does a different thing.
#[test]
fn the_key_hook_sees_every_key_first() {
    /// Swallows `x`, rewrites `!`, and lets everything else through.
    struct Hook;
    impl Assist for Hook {
        fn watches_keys(&mut self) -> bool {
            true
        }
        fn key_hook(&mut self, key: Key, line: &str, _cursor: usize) -> Option<KeyHook> {
            match key {
                Key::Char('x') => Some(KeyHook::Swallow),
                Key::Char('!') => Some(KeyHook::Line {
                    text: format!("sudo {line}"),
                    cursor: 0,
                    submit: false,
                }),
                _ => None,
            }
        }
        // Bound too, to prove the hook is asked *before* this is.
        fn binding(&mut self, key: Key) -> Option<Bound> {
            (key == Key::Char('x')).then_some(Bound::ClearScreen)
        }
    }

    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    for key in typed("echo") {
        s.apply(key, &mut Hook);
    }
    assert_eq!(
        s.apply(Key::Char('x'), &mut Hook),
        Step::Continue { redraw: false },
        "a swallowed key beats even a binding on the same key"
    );
    assert_eq!(s.buffer.text(), "echo", "and never reaches the buffer");

    assert_eq!(
        s.apply(Key::Char('!'), &mut Hook),
        Step::Continue { redraw: true }
    );
    assert_eq!(s.buffer.text(), "sudo echo", "the hook replaced the line");
    assert_eq!(s.buffer.cursor(), 0, "and placed the cursor");
}

/// A hook that declines leaves the key doing exactly what it did before — including a key the
/// config bound, which the hook is asked about first but has no opinion on.
#[test]
fn a_declining_key_hook_changes_nothing() {
    struct Quiet(usize);
    impl Assist for Quiet {
        fn watches_keys(&mut self) -> bool {
            true
        }
        fn key_hook(&mut self, _key: Key, _line: &str, _cursor: usize) -> Option<KeyHook> {
            self.0 += 1;
            None
        }
    }

    let mut seen = Quiet(0);
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    for key in typed("ls -l") {
        s.apply(key, &mut seen);
    }
    assert_eq!(
        s.apply(Key::Ctrl('w'), &mut seen),
        Step::Continue { redraw: true }
    );
    assert_eq!(s.buffer.text(), "ls ", "C-w still killed the word");
    assert_eq!(seen.0, 6, "and the hook was asked about every one of them");
}

/// **Nothing is asked when nothing is attached.** `key_hook` is the only `Assist` method on the
/// path of ordinary typing, so a session with no handler must not even build the line to offer.
#[test]
fn an_unwatched_session_never_builds_the_payload() {
    struct Never;
    impl Assist for Never {
        fn key_hook(&mut self, _key: Key, _line: &str, _cursor: usize) -> Option<KeyHook> {
            panic!("asked despite watches_keys() being false");
        }
    }
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    for key in typed("echo hi") {
        s.apply(key, &mut Never);
    }
    assert_eq!(s.buffer.text(), "echo hi");
}

/// A hook may run the line, which is what makes it able to replace a binding outright.
#[test]
fn the_key_hook_can_submit() {
    struct Go;
    impl Assist for Go {
        fn watches_keys(&mut self) -> bool {
            true
        }
        fn key_hook(&mut self, _key: Key, _line: &str, _cursor: usize) -> Option<KeyHook> {
            Some(KeyHook::Line {
                text: "ll".to_string(),
                cursor: 2,
                submit: true,
            })
        }
    }
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    assert_eq!(s.apply(Key::Ctrl('o'), &mut Go), Step::Accept);
    assert_eq!(s.buffer.text(), "ll");
}

#[test]
fn multiline_paste_inserts_without_submitting() {
    let mut session = Session {
        vi: None,
        ..Session::new("echo ", 5)
    };
    assert_eq!(
        session.paste("one\necho two"),
        Step::Continue { redraw: true }
    );
    assert_eq!(session.buffer.text(), "echo one\necho two");
}

#[path = "tests/taking.rs"]
mod taking;
