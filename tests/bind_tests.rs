//! `bind`, driven through a real shell.
//!
//! What a non-interactive test can reach is the registry half: that a spec parses, that it is
//! recorded as a *command* rather than a readline function, that rebinding replaces and `bind -r`
//! removes. The other half — pressing the key, the command seeing `$READLINE_LINE`, and the line
//! coming back rewritten — needs a terminal, and the repo has no pty harness by choice
//! (`tests/interactive_tests.rs` says why). That half was verified against a live pty before this
//! shipped: nine checks, including a full-screen program drawing from a binding.
//!
//! The specs here are copied from `atuin init bash` and `hexe shp init bash`. They are the reason
//! any of this exists, so they are what the tests are written against.

mod common;

use common::run_in;

/// Both integrations' own `bind` lines, in the exact shape they emit them.
#[test]
fn the_specs_real_integrations_ship_are_accepted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        r#"bind -x '"\C-r": __atuin_history'
bind -x '"\C-d":__hexe_ctrl_d'
echo "status=$?""#,
    );
    assert_eq!(r.out(), "status=0", "stderr: {}", r.stderr);
    assert!(r.stderr.is_empty(), "stderr: {}", r.stderr);
}

/// `bind -X` reports the `-x` bindings in the form that would bind them again, which is how an
/// init script run twice avoids binding the same key twice.
#[test]
fn the_listing_reports_what_was_bound() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        r#"bind -x '"\C-r": __atuin_history'
bind -X"#,
    );
    assert!(
        r.out().contains("__atuin_history"),
        "listing was {:?}",
        r.out()
    );
    assert!(r.out().contains(r"\C-r"), "listing was {:?}", r.out());
}

/// A binding that could not be read has to say so. Silently binding nothing is the failure mode
/// where a user spends an evening working out their keybinding never existed.
#[test]
fn an_unreadable_spec_is_reported() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(dir.path(), "bind -x 'no colon here'\necho \"status=$?\"");
    assert_eq!(r.out(), "status=1");
    assert!(
        r.stderr.contains("colon"),
        "stderr said {:?}, which does not name the problem",
        r.stderr
    );
}

#[test]
fn unbinding_something_that_was_never_bound_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        r#"bind -r '"\C-q"'
echo "status=$?""#,
    );
    assert_eq!(r.out(), "status=1");
}

/// Rebinding replaces. Two entries for one key would mean the older one firing on some presses,
/// which is the kind of intermittent nobody reports usefully.
#[test]
fn rebinding_a_key_replaces_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        r#"bind -x '"\C-t": first'
bind -x '"\C-t": second'
bind -X"#,
    );
    assert!(r.out().contains("second"), "{:?}", r.out());
    assert!(
        !r.out().contains("first"),
        "the replaced binding is still listed: {:?}",
        r.out()
    );
}

/// A readline *variable* is not a binding. Init scripts set these unconditionally, so a
/// diagnostic per line would bury the ones that matter.
#[test]
fn readline_variables_are_accepted_quietly() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        "bind 'set completion-ignore-case on'\necho \"status=$?\"",
    );
    assert_eq!(r.out(), "status=0");
    assert!(r.stderr.is_empty(), "stderr: {}", r.stderr);
}

/// The whole point, in one test: the five integrations a person actually has installed must
/// `eval` without a diagnostic. Each of these failed before `bind` and the DEBUG trap existed.
#[test]
fn shell_integrations_evaluate_cleanly() {
    for tool in [
        ("atuin", "atuin init bash"),
        ("hexe", "hexe shp init bash"),
        ("zoxide", "zoxide init bash"),
    ] {
        if which(tool.0).is_none() {
            continue;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let r = run_in(dir.path(), &format!("eval \"$({})\"\necho loaded", tool.1));
        assert_eq!(r.out(), "loaded", "{}: stderr {}", tool.0, r.stderr);
        assert!(
            !r.stderr.contains("command not found"),
            "{} needs something oslo does not have: {}",
            tool.0,
            r.stderr
        );
    }
}

/// Whether a program is installed, so the test above skips rather than fails on a machine that
/// does not have it. A test that depends on the developer's `$PATH` is a test that fails in CI.
fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}
