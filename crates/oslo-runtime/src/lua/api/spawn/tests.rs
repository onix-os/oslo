//! Running a process and reading what it produced. Delivering the result into Lua needs an
//! interpreter and is covered by `tests/lua_corpus/spawn.lua`, through the real binary.

use super::*;

#[test]
fn a_command_answers_its_output_and_status() {
    let (out, status) = run("printf", &["hello".to_string()], None);
    assert_eq!(out, "hello");
    assert_eq!(status, 0);
}

#[test]
fn a_failing_command_answers_its_status_rather_than_pretending() {
    let (_, status) = run("false", &[], None);
    assert_ne!(status, 0);
}

/// 127 is what a shell answers for a command it could not run, so a callback reading the status
/// sees a number it already knows the meaning of.
#[test]
fn a_command_that_is_not_there_is_127() {
    let (out, status) = run("definitely-not-a-command-anywhere", &[], None);
    assert_eq!(status, 127);
    assert!(out.is_empty());
}

/// **The one that hangs if the pipes are read in the wrong order.** More output than a pipe buffer
/// holds, which is the failure `nix_shell::json` has its own test for.
#[test]
fn more_output_than_a_pipe_buffer_arrives_whole() {
    let (out, status) = run(
        "awk",
        &["BEGIN { while (i++ < 4096) printf \"%064d\", i }".to_string()],
        None,
    );
    assert_eq!(status, 0);
    assert_eq!(out.len(), 256 * 1024);
}

/// A background job nobody waits for is exactly the kind never noticed hanging.
#[test]
fn a_command_that_overruns_is_killed_and_reports_124() {
    let started = std::time::Instant::now();
    let (_, status) = run(
        "sleep",
        &["30".to_string()],
        Some(Duration::from_millis(150)),
    );
    assert_eq!(status, 124, "the `timeout(1)` convention");
    assert!(started.elapsed() < Duration::from_secs(10), "not killed");
}

/// The callback is looked up by id and forgotten once handed back, so nothing fires twice.
#[test]
fn cancelling_forgets_the_callback() {
    let nothing = super::super::util::native("nothing", |_, _| Ok(Vec::new()));
    WAITING.with(|slot| slot.borrow_mut().insert(99, nothing));
    let Value::Table(handle) = handle(99) else {
        panic!("no handle")
    };
    let Value::Function(cancel) = handle.borrow().get(&Value::str("cancel")) else {
        panic!("no cancel")
    };
    let call = || match &*cancel {
        oslo_lua::value::Function::Native { call, .. } => {
            let interp = oslo_lua::Interp::new("test");
            call(&interp, Vec::new()).expect("cancel")
        }
        _ => panic!("not native"),
    };
    assert_eq!(call().first().map(Value::truthy), Some(true));
    // Twice says there was nothing left to cancel.
    assert_eq!(call().first().map(Value::truthy), Some(false));
}
