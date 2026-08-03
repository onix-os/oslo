//! Readline key syntax, checked against the specs real integrations ship.
//!
//! The literal strings here were taken from `atuin init bash` and `hexe shp init bash` rather than
//! invented, because the syntax is only worth reading to the extent those parse.

use super::*;
use std::sync::{MutexGuard, OnceLock};

/// The registry is one per process, because a shell has one line editor. Every test below writes
/// to it and libtest runs them at once, so they take turns — without this they clear each other's
/// bindings, which looks exactly like a bug in the code under test.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    let guard = guard.unwrap_or_else(|poisoned| poisoned.into_inner());
    clear();
    guard
}

fn one(spec: &str) -> KeyEvent {
    let events = parse_sequence(spec).expect("parses");
    assert_eq!(events.len(), 1, "{spec} is one key");
    events[0]
}

/// The two specs that started this: atuin's Ctrl-R and hexe's Ctrl-D.
#[test]
fn the_specs_shipped_by_real_integrations_parse() {
    assert_eq!(one(r#""\C-r""#), KeyEvent::ctrl('r'));
    assert_eq!(one(r#""\C-d""#), KeyEvent::ctrl('d'));
    // Written without the quotes just as often.
    assert_eq!(one(r"\C-r"), KeyEvent::ctrl('r'));
}

/// An arrow key arrives as the sequence the terminal sends, and has to be read as the key.
#[test]
fn escape_sequences_are_the_keys_they_stand_for() {
    assert_eq!(one(r#""\e[A""#), KeyEvent(KeyCode::Up, Modifiers::NONE));
    assert_eq!(one(r#""\e[B""#), KeyEvent(KeyCode::Down, Modifiers::NONE));
    assert_eq!(one(r#""\e[C""#), KeyEvent(KeyCode::Right, Modifiers::NONE));
    assert_eq!(one(r#""\e[D""#), KeyEvent(KeyCode::Left, Modifiers::NONE));
    assert_eq!(
        one(r#""\e[3~""#),
        KeyEvent(KeyCode::Delete, Modifiers::NONE)
    );
    assert_eq!(one(r#""\eOA""#), KeyEvent(KeyCode::Up, Modifiers::NONE));
}

#[test]
fn meta_is_alt() {
    assert_eq!(one(r#""\M-a""#), KeyEvent::alt('a'));
    // `\ea` with no bracket is Alt too, which is how readline writes it.
    assert_eq!(one(r#""\ea""#), KeyEvent::alt('a'));
}

/// Ctrl-I *is* Tab as far as a terminal is concerned. Binding it as `ctrl-i` would never fire,
/// so the ambiguous three are resolved to the key the terminal actually delivers.
#[test]
fn the_keys_a_terminal_cannot_tell_apart_resolve_to_what_it_sends() {
    assert_eq!(one(r#""\C-i""#), KeyEvent(KeyCode::Tab, Modifiers::NONE));
    assert_eq!(one(r#""\C-m""#), KeyEvent(KeyCode::Enter, Modifiers::NONE));
    assert_eq!(one(r#""\C-[""#), KeyEvent(KeyCode::Esc, Modifiers::NONE));
    assert_eq!(
        one(r#""\C-?""#),
        KeyEvent(KeyCode::Backspace, Modifiers::NONE)
    );
    assert_eq!(one(r#""\t""#), KeyEvent(KeyCode::Tab, Modifiers::NONE));
}

#[test]
fn a_sequence_is_more_than_one_key() {
    let events = parse_sequence(r#""\C-x\C-r""#).expect("parses");
    assert_eq!(events, vec![KeyEvent::ctrl('x'), KeyEvent::ctrl('r')]);
    let events = parse_sequence(r#""ab""#).expect("parses");
    assert_eq!(
        events,
        vec![
            KeyEvent(KeyCode::Char('a'), Modifiers::NONE),
            KeyEvent(KeyCode::Char('b'), Modifiers::NONE),
        ]
    );
}

#[test]
fn an_empty_spec_is_not_a_key() {
    assert_eq!(parse_sequence(""), None);
    assert_eq!(parse_sequence(r#""""#), None);
}

/// Rebinding a key replaces the old binding rather than stacking a second one, and `bind -r`
/// takes it off. Both go through the same key parse, so `"\C-r"` and `\C-r` name the same key.
#[test]
fn binding_is_by_key_not_by_spelling() {
    let _lock = exclusive();
    bind(r#""\C-t""#, Keymap::Emacs, Bound::Command("first".into())).expect("binds");
    bind(r"\C-t", Keymap::Emacs, Bound::Command("second".into())).expect("rebinds");
    let entries = entries();
    let ours: Vec<_> = entries
        .iter()
        .filter(|e| e.keys == vec![KeyEvent::ctrl('t')])
        .collect();
    assert_eq!(ours.len(), 1, "one binding for one key");
    assert_eq!(ours[0].bound, Bound::Command("second".into()));

    assert!(unbind(r#""\C-t""#, Keymap::Emacs));
    assert!(!unbind(r#""\C-t""#, Keymap::Emacs), "already gone");
}

/// The generation counter is what tells the read loop to re-apply, so every change must move it.
#[test]
fn every_change_moves_the_generation() {
    let _lock = exclusive();
    let start = generation();
    bind(r#""\C-t""#, Keymap::Emacs, Bound::Command("x".into())).expect("binds");
    assert!(generation() > start);
    let after_bind = generation();
    unbind(r#""\C-t""#, Keymap::Emacs);
    assert!(generation() > after_bind);
}

/// The request survives a round trip intact — this is the whole channel between the editor and
/// the read loop, and a dropped cursor position is a plugin that pastes in the wrong place.
#[test]
fn a_request_round_trips() {
    let _lock = exclusive();
    let _ = take_request();
    assert_eq!(take_request(), None);
    request(vec!["__atuin_history".to_string()], "git comm", 8);
    let taken = take_request().expect("a request");
    assert_eq!(taken.commands, vec!["__atuin_history".to_string()]);
    assert_eq!(taken.line, "git comm");
    assert_eq!(taken.point, 8);
    assert_eq!(take_request(), None, "taken exactly once");
}

/// A macro expands into the commands its key sequence is bound to, in order.
///
/// This is atuin's chain in miniature: a key stands for a sequence, each part of the sequence is a
/// `bind -x` command, and pressing the key has to run all of them. Before macros existed the
/// binding was recorded and nothing happened at all.
#[test]
fn a_macro_expands_to_the_commands_underneath_it() {
    let _lock = exclusive();
    bind(
        r#""\C-x\C-_A1\a""#,
        Keymap::Emacs,
        Bound::Command("first".into()),
    )
    .expect("binds");
    bind(
        r#""\C-x\C-_A2\a""#,
        Keymap::Emacs,
        Bound::Command("second".into()),
    )
    .expect("binds");
    let expansion = parse_sequence(r#""\C-x\C-_A1\a\C-x\C-_A2\a""#).expect("parses");
    bind(
        r#""\C-r""#,
        Keymap::Emacs,
        Bound::Macro {
            keys: expansion.clone(),
            text: String::new(),
        },
    )
    .expect("binds");

    assert_eq!(expand(&expansion), vec!["first", "second"]);
}

/// The bound sequences are not prefix-free — atuin binds both `A1` and `A10` — so the longest
/// match has to win. Taking the shorter one runs the wrong widget and leaves stray keys behind.
#[test]
fn the_longest_bound_sequence_wins() {
    let _lock = exclusive();
    bind(
        r#""\C-x\C-_A1\a""#,
        Keymap::Emacs,
        Bound::Command("short".into()),
    )
    .expect("binds");
    bind(
        r#""\C-x\C-_A10\a""#,
        Keymap::Emacs,
        Bound::Command("long".into()),
    )
    .expect("binds");
    let keys = parse_sequence(r#""\C-x\C-_A10\a""#).expect("parses");
    assert_eq!(expand(&keys), vec!["long"]);
}

/// Keys nothing is bound to are skipped rather than reported: a macro is a key sequence, and a
/// key with no binding is an ordinary keypress with nothing to do.
#[test]
fn unbound_keys_in_a_macro_are_skipped() {
    let _lock = exclusive();
    bind(r#""\C-t""#, Keymap::Emacs, Bound::Command("only".into())).expect("binds");
    let keys = parse_sequence(r#""z\C-tz""#).expect("parses");
    assert_eq!(expand(&keys), vec!["only"]);
}

/// A macro that reaches itself must stop rather than recurse. `bind '"\C-a": "\C-a"'` is a
/// single line anyone could type.
#[test]
fn a_macro_that_loops_stops() {
    let _lock = exclusive();
    let keys = parse_sequence(r#""\C-a""#).expect("parses");
    bind(
        r#""\C-a""#,
        Keymap::Emacs,
        Bound::Macro {
            keys: keys.clone(),
            text: String::new(),
        },
    )
    .expect("binds");
    // The assertion is that this returns at all.
    let commands = expand(&keys);
    assert!(commands.is_empty(), "{commands:?}");
}
