//! The standard library's own answers, where oslo's used to differ from Lua's.
//!
//! Split from `lua_eval_tests.rs`, which is about the *evaluator* — statements, scoping, the
//! numeric tower. These are about what the library functions return, and they were the file's
//! largest single group.

use oslo::lua::eval;

/// Run a chunk and collect what it returned, rendered as Lua would print it.
///
/// The same two helpers `lua_eval_tests` uses, and deliberately a copy rather than something moved
/// into `common`: they are four lines, and `common` is the *process*-level harness — putting an
/// in-process evaluator beside it would invite a test to reach for the wrong one.
fn eval_to_string(source: &str) -> Result<String, String> {
    eval::run(source, "test")
        .map(|values| {
            values
                .iter()
                .map(|v| v.to_display())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .map_err(|e| e.to_string())
}

/// Assert that `expr` evaluates to `expected`.
#[track_caller]
fn returns(expr: &str, expected: &str) {
    let source = format!("return {expr}");
    match eval_to_string(&source) {
        Ok(got) => assert_eq!(got, expected, "for `{expr}`"),
        Err(e) => panic!("`{expr}` failed: {e}"),
    }
}

/// **`os.time` with a table answers the date it was given**, not the current one.
///
/// It used to return `os.time()` whatever the table said — a plausible number and the wrong one,
/// with nothing to say so. The reason recorded in the source was that a calendar is a dependency
/// the shell does not carry; the calendar was already there, in `os.date`, and this is its inverse.
/// UTC throughout, as `os.date` is.
#[test]
fn os_time_reads_the_table_it_was_given() {
    returns("os.time{year=1970, month=1, day=1, hour=0}", "0");
    returns("os.time{year=2000, month=1, day=1, hour=0}", "946684800");
    // Lua's default hour is midday, not midnight.
    returns("os.time{year=2000, month=1, day=1}", "946728000");
    // The leap day exists.
    returns("os.time{year=2024, month=2, day=29, hour=0}", "1709164800");
    // And it round-trips through the formatter.
    returns(
        "os.date('!%Y-%m-%d', os.time{year=2024, month=2, day=29, hour=0})",
        "2024-02-29",
    );
}

/// **`error(msg, 0)` means "no position", and `assert` never had one.**
///
/// Level 0 is how a library says the message is the whole error. It was ignored, so the message
/// arrived at a handler wearing a `file:line:` it had asked not to wear — which matters because
/// reading one back is `message:match(":(%d+):")`, and a message that answers when it should not
/// is worse than one that never does. `assert` raises its message as the error *object* rather
/// than through `error`, so Lua never puts a position in front of it either.
#[test]
fn error_level_zero_and_assert_carry_no_position() {
    returns(
        "select(2, pcall(function() error('plain', 0) end))",
        "plain",
    );
    returns(
        "select(2, pcall(function() assert(false, 'my message') end))",
        "my message",
    );
    returns(
        "select(2, pcall(function() assert(false) end))",
        "assertion failed!",
    );
    // The default level still names where it happened, which is the whole point of the default.
    returns(
        "(select(2, pcall(function() error('located') end))):match(':%d+: (.*)')",
        "located",
    );
}
