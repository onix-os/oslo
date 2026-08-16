//! The standard library's own answers, and the three names oslo refuses.
//!
//! Split from `lua_eval_tests.rs`, which is about the *language* — statements, scoping, closures,
//! metamethods, the numeric tower. These are about what the library returns: `string`, `table`,
//! `math`, `os`, and the patterns. They were that file's largest single group.
//!
//! The refusals live here rather than there for the same reason. `os.execute`, `io.popen` and
//! `os.tmpname` are library functions the VM implements correctly and oslo replaces on purpose —
//! which makes them the one group in this file that has to run in a real process, because the
//! replacement happens when the shell builds its `oslo` table.

mod common;

use oslo_luavm::Engine;

/// Run a chunk through the real binary, which is the only place oslo's own surface exists.
///
/// **Most cases here do not need this.** What does is anything about the names oslo *replaces*:
/// `os.execute`, `io.popen` and `os.tmpname` are refused by `lua::api::policy`, which is installed
/// when the shell builds its `oslo` table and nowhere else — a bare [`Engine`] has the VM's own
/// working versions.
fn in_the_shell(source: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("case.lua");
    std::fs::write(&script, source).expect("write");
    let out = std::process::Command::new(common::oslo_bin())
        .arg(&script)
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn oslo");
    String::from_utf8_lossy(&out.stdout).trim_end().to_string()
}

/// Run a chunk and collect what it returned, rendered as Lua would print it.
///
/// The same two helpers `lua_eval_tests` uses, and deliberately a copy rather than something moved
/// into `common`: they are four lines, and `common` is the *process*-level harness — putting an
/// in-process evaluator beside it would invite a test to reach for the wrong one.
fn eval_to_string(source: &str) -> Result<String, String> {
    Engine::new()
        .eval(source, "test")
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

/// **A name oslo refuses is a function that explains itself, never a `nil`.**
///
/// The rule used to be about a tree walker's gaps — `coroutine.create` was a stub that raised
/// rather than a missing field, so the first use said what was wrong instead of `attempt to call a
/// nil value`. The VM implements coroutines, so the gaps are gone; what survives the rule is the
/// handful of names oslo replaces *on purpose*, and they are held to it exactly the same way.
#[test]
fn a_refused_name_is_present_and_says_so() {
    for (call, expected) in [
        ("os.execute('ls')", "oslo.run"),
        ("io.popen('ls')", "oslo.run"),
        ("os.tmpname()", "oslo.fs.mktemp"),
    ] {
        let name = call.split('(').next().unwrap();
        assert_eq!(
            in_the_shell(&format!("print(type({name}))")),
            "function",
            "{name} is not there at all, so the first use is a nil error"
        );
        let out = in_the_shell(&format!(
            "local ok, err = pcall(function() return {call} end) print(ok, err)"
        ));
        assert!(out.starts_with("false\t"), "for {call}: {out}");
        assert!(out.contains(expected), "for {call}: {out}");
    }
}

#[test]
fn tostring_and_tonumber_round_trip() {
    returns("tostring(3)", "3");
    returns("tostring(3.0)", "3.0");
    returns("tostring(nil)", "nil");
    returns("tonumber('0x10')", "16");
    returns("tonumber('10', 2)", "2");
    returns("tonumber('abc')", "nil");
}

#[test]
fn tostring_metamethod_wins() {
    let source = r"
        local t = setmetatable({}, {__tostring = function() return 'custom' end})
        return tostring(t)
    ";
    assert_eq!(eval_to_string(source).unwrap(), "custom");
}

#[test]
fn string_methods_are_reachable_through_the_value() {
    // `("x"):upper()` works because indexing a string falls back to the string library.
    returns("('abc'):upper()", "ABC");
    returns("('abc'):len()", "3");
    returns("string.sub('hello', 2, 4)", "ell");
    // A negative index counts back from the end.
    returns("('hello'):sub(-3)", "llo");
    returns("('ab'):rep(3, '-')", "ab-ab-ab");
    returns("string.byte('A')", "65");
    returns("string.char(104, 105)", "hi");
}

#[test]
fn string_format_follows_c_conventions() {
    returns("string.format('%d items', 3)", "3 items");
    returns("string.format('%5.2f', 3.14159)", " 3.14");
    returns("string.format('%05d', -42)", "-0042");
    returns("string.format('%-4d|', 7)", "7   |");
    returns("string.format('%x', 255)", "ff");
    returns("string.format('%s and %s', 'a', 'b')", "a and b");
    returns("string.format('100%%')", "100%");
}

#[test]
fn patterns_drive_find_match_and_gsub() {
    returns("string.match('key=value', '(%w+)=(%w+)')", "key\tvalue");
    returns("string.find('hello', 'l')", "3\t3");
    // A plain search takes the pattern literally, punctuation and all.
    returns("string.find('a.c', '.', 1, true)", "2\t2");
    returns("string.gsub('hello world', 'o', '0')", "hell0 w0rld\t2");
    returns("string.gsub('hello', 'l', 'L', 1)", "heLlo\t1");
    returns("select('#', ('a,b,c'):gsub(',', ';'))", "2");

    // **`^` anchors the call, not every attempt.** The matcher applies the anchor wherever it is
    // asked to start, which is right for `find` and wrong for a `gsub` that walks forward: every
    // position looked like the beginning of the subject, so this answered `XXX` and replaced both
    // halves of `abcabc`. Lua 5.4 stops after one attempt when the pattern is anchored.
    returns("string.gsub('aaa', '^a', 'X')", "Xaa\t1");
    returns("string.gsub('abcabc', '^abc', '-')", "-abc\t1");
    returns("string.gsub('hello world', '^hello', 'HI')", "HI world\t1");
    // A pattern that does not match at the start replaces nothing at all.
    returns("string.gsub('xabc', '^abc', '-')", "xabc\t0");
    // `find` keeps anchoring at the position it was given, which is Lua's behaviour too.
    returns("string.find('aaa', '^a', 2)", "2\t2");

    let counted = r"
        local words = {}
        for w in ('the quick fox'):gmatch('%a+') do words[#words + 1] = w end
        return #words, words[2]
    ";
    assert_eq!(eval_to_string(counted).unwrap(), "3\tquick");

    let by_function = r"
        return (('abc'):gsub('%a', function(c) return c:upper() end))
    ";
    assert_eq!(eval_to_string(by_function).unwrap(), "ABC");
}

#[test]
fn table_library_covers_the_sequence_operations() {
    let source = r"
        local t = {'a', 'c'}
        table.insert(t, 2, 'b')
        table.insert(t, 'd')
        local removed = table.remove(t, 1)
        return table.concat(t, ','), removed, #t
    ";
    assert_eq!(eval_to_string(source).unwrap(), "b,c,d\ta\t3");

    returns("table.concat({1, 2, 3}, '-')", "1-2-3");
    returns("table.unpack({1, 2, 3})", "1\t2\t3");
    returns("table.pack(1, nil, 3).n", "3");
}

#[test]
fn sort_uses_the_comparator_it_is_given() {
    let source = r"
        local t = {3, 1, 2}
        table.sort(t)
        local ascending = table.concat(t, '')
        table.sort(t, function(a, b) return a > b end)
        return ascending, table.concat(t, '')
    ";
    assert_eq!(eval_to_string(source).unwrap(), "123\t321");

    // A comparator that raises must surface the error, not be swallowed or panic.
    let failing = "local ok = pcall(table.sort, {2, 1}, function() error('no') end) return ok";
    assert_eq!(eval_to_string(failing).unwrap(), "false");
}

#[test]
fn math_keeps_the_integer_float_distinction() {
    // `floor` produces an integer precisely so its result can index a table.
    returns("math.floor(3.7)", "3");
    returns("math.type(math.floor(3.7))", "integer");
    returns("math.ceil(3.2)", "4");
    returns("math.sqrt(4)", "2.0");
    returns("math.max(1, 5, 3)", "5");
    returns("math.min(1, 5, 3)", "1");
    returns("math.abs(-7)", "7");
    returns("math.tointeger(3.0)", "3");
    returns("math.tointeger(3.5)", "nil");
    returns("math.type(1)", "integer");
    returns("math.type(1.0)", "float");
    returns("math.type('1')", "nil");
    returns("math.huge > math.maxinteger", "true");
}

#[test]
fn random_stays_inside_the_range_it_was_given() {
    let source = r"
        math.randomseed(1)
        local low, high = 1, 6
        for _ = 1, 200 do
            local roll = math.random(low, high)
            if roll < low or roll > high then return 'out of range: ' .. roll end
            if math.type(roll) ~= 'integer' then return 'not an integer' end
        end
        local unit = math.random()
        if unit < 0 or unit >= 1 then return 'unit out of range' end
        return 'ok'
    ";
    assert_eq!(eval_to_string(source).unwrap(), "ok");
}

#[test]
fn os_date_formats_the_directives_a_script_uses() {
    // A fixed timestamp, so the assertion is about the arithmetic and not about today.
    returns("os.date('%Y-%m-%d', 1709164800)", "2024-02-29");
    returns("os.date('%H:%M:%S', 1709210096)", "12:34:56");
    // The day of the year has to count the leap day that came before it.
    returns("os.date('%j', 1709164800)", "060");
    returns("os.date('%Y%%', 0)", "1970%");
    // The name directives, which the evaluator this replaced passed through untranslated.
    returns("os.date('%A', 0)", "Thursday");
    returns("os.date('%b', 0)", "Jan");
}

#[test]
fn the_two_shell_out_routes_refuse_and_say_what_to_use() {
    // Real Lua runs both through `/bin/sh` — someone else's shell, from inside this one, and
    // nothing at all on a system where oslo is the only shell installed. The VM implements both
    // correctly; oslo replaces them, so this has to run where that replacement happened.
    for call in ["os.execute('ls')", "io.popen('ls')"] {
        let message = in_the_shell(&format!(
            "local ok, err = pcall(function() return {call} end) print(err)"
        ));
        assert!(message.contains("/bin/sh"), "for {call}: {message}");
        assert!(message.contains("oslo.run"), "for {call}: {message}");
    }
    // With no argument `os.execute()` asks whether a shell is available, and one is.
    assert_eq!(in_the_shell("print(os.execute())"), "true");
}

#[test]
fn os_tmpname_points_at_the_call_that_is_not_a_race() {
    let message =
        in_the_shell("local ok, err = pcall(function() return os.tmpname() end) print(err)");
    assert!(message.contains("oslo.fs.mktemp"), "{message}");
}

#[test]
fn os_getenv_reads_the_real_environment() {
    // SAFETY: the test process, before anything else reads this name.
    unsafe { std::env::set_var("OSLO_EVAL_TEST_VAR", "present") };
    returns("os.getenv('OSLO_EVAL_TEST_VAR')", "present");
    returns("os.getenv('OSLO_DEFINITELY_UNSET_VAR_ZZ')", "nil");
}
