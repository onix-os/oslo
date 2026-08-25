//! What a *handler* does to a key: the `oslo.keys` route and the `key` hook.
//!
//! Split from [`super`] because these are the only cases that need an [`Assist`] of their own —
//! each one stands up a struct answering a fixed [`Placed`] — while the rest of the state machine
//! is checked against `NoAssist` and a bare line.

use super::super::*;
use super::typed;

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
        fn lua_key(&mut self, _n: &str, _l: &str, _c: usize) -> Option<Placed> {
            Some(Placed {
                text: " _a".to_string(),
                cursor: 3,
                submit: self.submit,
                erase: false,
            })
        }
    }

    let mut s = Session {
        vi: None,
        ..Session::new("half typed", 10)
    };
    assert_eq!(
        s.apply(Key::Alt('a'), &mut Bind { submit: true }),
        Step::Accept { erase: false },
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
                Key::Char('!') => Some(KeyHook::Line(Placed {
                    text: format!("sudo {line}"),
                    cursor: 0,
                    submit: false,
                    erase: false,
                })),
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
            Some(KeyHook::Line(Placed {
                text: "ll".to_string(),
                cursor: 2,
                submit: true,
                erase: false,
            }))
        }
    }
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    assert_eq!(
        s.apply(Key::Ctrl('o'), &mut Go),
        Step::Accept { erase: false }
    );
    assert_eq!(s.buffer.text(), "ll");
}

/// **The erase rides all the way to the step.** A hook that runs a line without leaving it behind
/// is only useful if the draw loop is told; the flag went missing between the two once already,
/// and the symptom is a prompt stacking up one row per keypress.
#[test]
fn a_submitting_hook_can_ask_for_its_line_to_be_erased() {
    struct Quietly(bool);
    impl Assist for Quietly {
        fn watches_keys(&mut self) -> bool {
            true
        }
        fn key_hook(&mut self, _key: Key, _line: &str, _cursor: usize) -> Option<KeyHook> {
            Some(KeyHook::Line(Placed {
                text: "nav".to_string(),
                cursor: 3,
                submit: true,
                erase: self.0,
            }))
        }
    }
    for erase in [true, false] {
        let mut s = Session {
            vi: None,
            ..Session::new("", 0)
        };
        assert_eq!(
            s.apply(Key::Char(' '), &mut Quietly(erase)),
            Step::Accept { erase }
        );
        assert_eq!(s.buffer.text(), "nav", "the line runs either way");
    }
}

/// **The line runs but is never drawn.** The whole point of the flag is that the prompt you are
/// looking at stays a prompt: putting `nav` on it for as long as the browser is up shows the word
/// the binding exists to spare you. What the editor hands back is still the line.
#[test]
fn an_erased_line_runs_without_being_shown() {
    struct Quietly;
    impl Assist for Quietly {
        fn watches_keys(&mut self) -> bool {
            true
        }
        fn key_hook(&mut self, _key: Key, _line: &str, _cursor: usize) -> Option<KeyHook> {
            Some(KeyHook::Line(Placed {
                text: "nav".to_string(),
                cursor: 3,
                submit: true,
                erase: true,
            }))
        }
    }
    let mut s = Session {
        vi: None,
        ..Session::new("", 0)
    };
    assert_eq!(
        s.apply(Key::Char(' '), &mut Quietly),
        Step::Accept { erase: true }
    );
    // The draw loop empties the buffer before its last frame, so what `apply` leaves behind is the
    // line — and the emptying is asserted where it happens, on `screen::park`'s side.
    assert_eq!(s.buffer.text(), "nav");
}
