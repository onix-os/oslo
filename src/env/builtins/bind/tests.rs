//! `bind`'s command-line surface.
//!
//! The specs asserted here are copied from `atuin init bash` and `hexe shp init bash`. If one of
//! them stops parsing, that integration stops having keys.

use super::*;
use crate::interactive::readline;
use rustyline::KeyEvent;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// The binding registry is one per process, because a shell has one line editor. These tests all
/// write to it, and libtest runs them on threads at once, so they take turns — without this they
/// fail by clearing each other's bindings, which looks exactly like a bug in `bind`.
fn exclusive() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = LOCK.get_or_init(|| Mutex::new(())).lock();
    let guard = guard.unwrap_or_else(|poisoned| poisoned.into_inner());
    readline::clear();
    guard
}

fn run(args: &[&str]) -> i32 {
    let mut env = Environment::new();
    let owned: Vec<String> = std::iter::once("bind".to_string())
        .chain(args.iter().map(|s| s.to_string()))
        .collect();
    builtin_bind(&mut env, &owned).expect("bind does not fail fatally")
}

#[test]
fn a_spec_splits_at_the_colon_after_the_key() {
    assert_eq!(
        split_spec(r#""\C-r": __atuin_history"#),
        Some((r#""\C-r""#.to_string(), "__atuin_history"))
    );
    // hexe writes it with no space after the colon.
    assert_eq!(
        split_spec(r#""\C-d":__hexe_ctrl_d"#),
        Some((r#""\C-d""#.to_string(), "__hexe_ctrl_d"))
    );
    assert_eq!(
        split_spec(r"\C-t: mycommand"),
        Some((r"\C-t".to_string(), "mycommand"))
    );
}

/// A key that is itself a colon must not split in the middle of itself.
#[test]
fn a_colon_inside_the_quoted_key_is_part_of_the_key() {
    assert_eq!(
        split_spec(r#""\C-x:": handler"#),
        Some((r#""\C-x:""#.to_string(), "handler"))
    );
}

#[test]
fn a_spec_without_an_action_is_not_a_binding() {
    assert_eq!(split_spec(r#""\C-r""#), None);
    assert_eq!(split_spec(r#""\C-r":"#), None);
    assert_eq!(split_spec(""), None);
}

/// End to end through the builtin, in the exact shape the two integrations emit.
#[test]
fn the_integrations_own_bind_lines_work() {
    let _lock = exclusive();
    assert_eq!(run(&["-x", r#""\C-r": __atuin_history"#]), 0);
    assert_eq!(run(&["-x", r#""\C-d":__hexe_ctrl_d"#]), 0);

    let entries = readline::entries();
    assert_eq!(entries.len(), 2);
    let ctrl_r = entries
        .iter()
        .find(|e| e.keys == vec![KeyEvent::ctrl('r')])
        .expect("ctrl-r is bound");
    assert_eq!(
        ctrl_r.bound,
        Bound::Command("__atuin_history".to_string()),
        "and it runs a command, not a readline function"
    );
}

/// `-x` binds the one spec after it. Without that, a later bare spec would silently become a
/// command binding and run arbitrary shell code on a keystroke nobody asked to be a command.
#[test]
fn the_x_flag_applies_to_one_spec() {
    let _lock = exclusive();
    run(&["-x", r#""\C-t": as_command"#, r#""\C-y": as_function"#]);
    let entries = readline::entries();
    let first = entries
        .iter()
        .find(|e| e.keys == vec![KeyEvent::ctrl('t')])
        .expect("bound");
    let second = entries
        .iter()
        .find(|e| e.keys == vec![KeyEvent::ctrl('y')])
        .expect("bound");
    assert_eq!(first.bound, Bound::Command("as_command".to_string()));
    assert_eq!(second.bound, Bound::Function("as_function".to_string()));
}

#[test]
fn unbinding_reports_whether_there_was_one() {
    let _lock = exclusive();
    run(&["-x", r#""\C-t": handler"#]);
    assert_eq!(run(&["-r", r#""\C-t""#]), 0);
    assert_eq!(run(&["-r", r#""\C-t""#]), 1, "nothing left to unbind");
}

/// A readline *variable* is not a binding. Init scripts set these unconditionally, so accepting
/// them quietly is what keeps a diagnostic meaningful when a real binding fails.
#[test]
fn readline_variables_are_accepted_and_ignored() {
    let _lock = exclusive();
    assert_eq!(run(&["set completion-ignore-case on"]), 0);
    assert_eq!(run(&["set editing-mode vi"]), 0);
    assert!(readline::entries().is_empty(), "nothing was bound");
}

#[test]
fn an_unreadable_spec_fails_rather_than_binding_nothing() {
    let _lock = exclusive();
    assert_eq!(run(&["-x", "no colon here"]), 1);
    assert_eq!(run(&["-x", r#""": handler"#]), 1);
    assert!(readline::entries().is_empty());
}
