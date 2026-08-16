//! End-to-end tests for oslo's own Lua evaluator.
//!
//! Each case is source in, printed output out — the same thing a user types. The point is to pin
//! behaviour that a reimplementation gets subtly wrong: integer versus float subtypes, what `#`
//! reports, when a metamethod fires, and which errors are catchable.

mod common;

use oslo_luavm::Engine;

/// Run a chunk through the real binary, which is the only place oslo's own surface exists.
///
/// **Most cases here do not need this.** They are language semantics — subtypes, `#`, metamethods —
/// and a bare [`Engine`] answers them in microseconds. What needs a process is anything about the
/// names oslo *replaces*: `os.execute`, `io.popen` and `os.tmpname` are refused by
/// `lua::api::policy`, which is installed when the shell builds its `oslo` table and nowhere else.
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

#[test]
fn arithmetic_keeps_lua_integer_and_float_subtypes() {
    returns("1 + 2", "3");
    // Division is always float, even when it divides evenly — this is the 5.3 change that breaks
    // the most ported code.
    returns("6 / 2", "3.0");
    returns("7 // 2", "3");
    returns("7.0 // 2", "3.0");
    returns("2 ^ 10", "1024.0");
    // Floor division and modulo follow the sign of the *divisor*, not the dividend.
    returns("-7 // 2", "-4");
    returns("-7 % 2", "1");
    returns("7 % -2", "-1");
}

#[test]
fn comparison_crosses_the_number_subtypes() {
    returns("1 == 1.0", "true");
    returns("'1' == 1", "false");
    returns("1 < 2", "true");
    returns("'a' < 'b'", "true");
}

#[test]
fn only_nil_and_false_are_false() {
    returns("not nil", "true");
    returns("not false", "true");
    // The two that catch everyone.
    returns("not 0", "false");
    returns("not ''", "false");
}

#[test]
fn concatenation_coerces_numbers_but_not_booleans() {
    returns("'a' .. 'b'", "ab");
    returns("1 .. 2", "12");
    assert!(eval_to_string("return true .. 'x'").is_err());
}

#[test]
fn length_reports_the_sequence_border() {
    returns("#'hello'", "5");
    returns("#{1, 2, 3}", "3");
    returns("#{}", "0");
}

#[test]
fn locals_shadow_and_assignment_finds_the_owner() {
    let source = r"
        local x = 1
        do
            local x = 2
            x = 3
        end
        return x
    ";
    assert_eq!(eval_to_string(source).unwrap(), "1");
}

#[test]
fn closures_capture_the_variable_not_its_value() {
    let source = r"
        local function counter()
            local n = 0
            return function()
                n = n + 1
                return n
            end
        end
        local c = counter()
        c()
        c()
        return c()
    ";
    assert_eq!(eval_to_string(source).unwrap(), "3");
}

#[test]
fn a_local_function_can_call_itself() {
    let source = r"
        local function fact(n)
            if n <= 1 then return 1 end
            return n * fact(n - 1)
        end
        return fact(5)
    ";
    assert_eq!(eval_to_string(source).unwrap(), "120");
}

#[test]
fn varargs_expand_only_in_the_last_position() {
    let source = r"
        local function pack(...)
            return select('#', ...)
        end
        return pack(1, 2, 3)
    ";
    assert_eq!(eval_to_string(source).unwrap(), "3");

    let truncated = r"
        local function two() return 1, 2 end
        local a, b, c = two(), 10
        return a, b, c
    ";
    // The non-final call is cut to one value; that is why `b` is 10 and not 2.
    assert_eq!(eval_to_string(truncated).unwrap(), "1\t10\tnil");
}

#[test]
fn numeric_for_binds_an_integer_when_its_bounds_are_integers() {
    let source = r"
        local t = {}
        for i = 1, 3 do t[i] = i * i end
        return t[1], t[2], t[3]
    ";
    assert_eq!(eval_to_string(source).unwrap(), "1\t4\t9");

    let downward = r"
        local sum = 0
        for i = 10, 1, -2 do sum = sum + i end
        return sum
    ";
    assert_eq!(eval_to_string(downward).unwrap(), "30");
}

#[test]
fn generic_for_walks_ipairs_and_pairs() {
    let source = r"
        local total = 0
        for _, v in ipairs({10, 20, 30}) do total = total + v end
        return total
    ";
    assert_eq!(eval_to_string(source).unwrap(), "60");

    let keys = r"
        local n = 0
        for k, v in pairs({a = 1, b = 2, c = 3}) do n = n + v end
        return n
    ";
    assert_eq!(eval_to_string(keys).unwrap(), "6");
}

#[test]
fn repeat_sees_its_bodys_locals_in_the_until_condition() {
    let source = r"
        local i = 0
        repeat
            local done = i >= 3
            i = i + 1
        until done
        return i
    ";
    assert_eq!(eval_to_string(source).unwrap(), "4");
}

#[test]
fn break_leaves_only_the_innermost_loop() {
    let source = r"
        local n = 0
        for i = 1, 3 do
            for j = 1, 10 do
                if j > 2 then break end
                n = n + 1
            end
        end
        return n
    ";
    assert_eq!(eval_to_string(source).unwrap(), "6");
}

#[test]
fn tables_are_shared_references() {
    let source = r"
        local a = {}
        local b = a
        b.x = 1
        return a.x
    ";
    assert_eq!(eval_to_string(source).unwrap(), "1");
}

#[test]
fn integral_float_keys_are_the_same_slot_as_integers() {
    let source = r"
        local t = {}
        t[2.0] = 'hit'
        return t[2]
    ";
    assert_eq!(eval_to_string(source).unwrap(), "hit");
}

#[test]
fn method_syntax_passes_self() {
    let source = r"
        local obj = {n = 7}
        function obj:get() return self.n end
        return obj:get()
    ";
    assert_eq!(eval_to_string(source).unwrap(), "7");
}

#[test]
fn index_metamethod_supports_inheritance() {
    let source = r"
        local base = {greet = function() return 'hi' end}
        local child = setmetatable({}, {__index = base})
        return child.greet()
    ";
    assert_eq!(eval_to_string(source).unwrap(), "hi");
}

#[test]
fn newindex_fires_only_for_absent_keys() {
    let source = r"
        local log = {}
        local t = setmetatable({existing = 1}, {
            __newindex = function(tbl, k, v) log[#log + 1] = k; rawset(tbl, k, v) end,
        })
        t.existing = 2
        t.fresh = 3
        return #log, log[1]
    ";
    assert_eq!(eval_to_string(source).unwrap(), "1\tfresh");
}

#[test]
fn call_metamethod_makes_a_table_callable() {
    let source = r"
        local t = setmetatable({}, {__call = function(_, x) return x * 2 end})
        return t(21)
    ";
    assert_eq!(eval_to_string(source).unwrap(), "42");
}

#[test]
fn pcall_catches_errors_and_lets_returns_through() {
    let source = r"
        local ok, err = pcall(function() error('boom') end)
        return ok, err
    ";
    let out = eval_to_string(source).unwrap();
    assert!(out.starts_with("false\t"), "got {out}");
    assert!(out.contains("boom"), "got {out}");

    // A `return` inside a pcall'd function is a return, not a failure — the reason `Flow` is a
    // separate type from `LuaError`.
    let returning = "return pcall(function() return 1, 2 end)";
    assert_eq!(eval_to_string(returning).unwrap(), "true\t1\t2");
}

/// The depth limit has to hold on the stack oslo actually gives itself, not on whatever the test
/// runner happened to provide — that is how a limit ships while still overflowing.
///
/// Both directions are checked here: recursion just under the limit must work, and recursion past
/// it must come back as a catchable error rather than SIGSEGV.
#[test]
fn recursion_is_bounded_by_a_catchable_error_on_the_stack_oslo_reserves() {
    let worker = std::thread::Builder::new()
        .stack_size(oslo::INTERPRETER_STACK)
        .spawn(|| {
            let deep = r"
                local function count(n)
                    if n <= 0 then return 0 end
                    return 1 + count(n - 1)
                end
                return count(150)
            ";
            assert_eq!(eval_to_string(deep).unwrap(), "150");

            // `tostring(err)` rather than `err`: the VM raises this one as userdata where Lua 5.4
            // raises a string, so the message is reachable but `err:find(…)` is not. What is being
            // pinned here is that the recursion *stops*, catchably, and says why — a runaway
            // function in a config must not take the shell down with it.
            let runaway = r"
                local function f() return 1 + f() end
                local ok, err = pcall(f)
                return ok, tostring(err)
            ";
            let out = eval_to_string(runaway).unwrap();
            assert!(out.starts_with("false\t"), "got {out}");
            assert!(out.contains("stack overflow"), "got {out}");
        })
        .expect("spawn");
    worker
        .join()
        .expect("the evaluator must not overflow the stack oslo reserves");
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

#[test]
fn incomplete_input_is_distinguishable_from_a_syntax_error() {
    assert!(oslo_luavm::is_complete("return 1"));
    assert!(!oslo_luavm::is_complete("if true then"));
    assert!(!oslo_luavm::is_complete("function f("));
    assert!(!oslo_luavm::is_complete("local t = {"));
    // A genuine mistake is complete-but-wrong: the prompt must report it, not wait for more.
    assert!(oslo_luavm::is_complete("return 1 +/ 2"));
    assert!(oslo_luavm::is_complete("x = = 2"));
}

#[test]
fn a_numeric_for_evaluates_each_bound_exactly_once() {
    // Asking for the number and then asking again whether it was an integer called `f` twice and
    // looped to the second answer.
    returns(
        "(function() local n = 0 \
           local function f() n = n + 1 return 3 end \
           for _ = 1, f() do end \
           return n end)()",
        "1",
    );
    returns(
        "(function() local n = 0 \
           local function g() n = n + 1 return 1 end \
           for _ = g(), 2, g() do end \
           return n end)()",
        "2",
    );
}

#[test]
fn a_numeric_for_still_binds_the_lua_subtypes() {
    returns(
        "(function() for i = 1, 1 do return math.type(i) end end)()",
        "integer",
    );
    returns(
        "(function() for i = 1.0, 1 do return math.type(i) end end)()",
        "float",
    );
    returns(
        "(function() for i = 1, 1, 1.0 do return math.type(i) end end)()",
        "float",
    );
}

#[test]
fn integers_compare_as_integers_rather_than_through_f64() {
    // Above 2^53 the two sides land on the same float, so the comparison used to answer false.
    returns("math.maxinteger - 1 < math.maxinteger", "true");
    returns("math.mininteger < math.mininteger + 1", "true");
    returns("math.maxinteger <= math.maxinteger", "true");
    // Mixed and float operands keep the f64 path.
    returns("1 < 2.5", "true");
    returns("2.5 <= 2.5", "true");
    returns("-1 < 0.5", "true");
}
