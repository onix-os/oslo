//! What a config has to say before a renderer is installed.

use super::*;
use oslo_luavm::{Engine, Host};

fn read(source: &str) -> Option<Spec> {
    let engine = Engine::new();
    engine
        .eval(source, "transcript test")
        .expect("the chunk runs");
    spec_of(&engine.global("oslo"))
}

/// **Nothing named, nothing installed.** A config with only a rule — or with none — must not end up
/// running a program between Enter and every command.
#[test]
fn a_command_has_to_be_named_to_be_run() {
    assert!(read("oslo = {}").is_none(), "no transcript at all");
    assert!(read("oslo = { transcript = {} }").is_none(), "no command");
    assert!(
        read(r#"oslo = { transcript = { rule = "- " } }"#).is_none(),
        "a rule is not a command"
    );
    assert!(
        read("oslo = { transcript = { command = {} } }").is_none(),
        "a table with nothing to run is nothing to run"
    );
}

/// The argv is read as written, and `$command` is the one thing substituted.
#[test]
fn the_argv_is_read_and_the_deadline_defaults_short() {
    let spec = read(
        r#"oslo = { transcript = { command = {
             command = "pixy",
             args = { "render", "transcript", "--set", "cmd=$command", 7 },
           } } }"#,
    )
    .expect("a spec");
    assert_eq!(spec.command, "pixy");
    assert_eq!(
        spec.args,
        vec!["render", "transcript", "--set", "cmd=$command", "7"],
        "numbers come through as words; everything else is dropped"
    );
    // **Short by default**, because this runs between Enter and the command starting: a frame is
    // worth a few milliseconds and not more.
    assert_eq!(spec.timeout, Duration::from_millis(20));

    let slow = read(r#"oslo = { transcript = { command = { command = "p", timeout_ms = 200 } } }"#)
        .expect("a spec");
    assert_eq!(slow.timeout, Duration::from_millis(200));
}
