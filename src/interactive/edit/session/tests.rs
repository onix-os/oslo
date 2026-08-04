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
    completion: Option<(String, usize)>,
}

impl Assist for Canned {
    fn complete(&mut self, _l: &str, _c: usize, _b: bool) -> Option<(String, usize)> {
        self.completion.clone()
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

/// Completion replaces the line and places the cursor where the completer asked.
#[test]
fn completion_replaces_the_line() {
    let mut a = Canned {
        completion: Some(("git checkout ".to_string(), 13)),
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("git ch", 6)
    };
    assert_eq!(
        s.apply(Key::ToggleScope, &mut a),
        Step::Continue { redraw: true }
    );
    assert_eq!(s.buffer.text(), "git checkout ");
    assert_eq!(s.buffer.cursor(), 13);
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
