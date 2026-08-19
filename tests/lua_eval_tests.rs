//! End-to-end tests for oslo's own Lua evaluator.
//!
//! Each case is source in, printed output out — the same thing a user types. The point is to pin
//! behaviour that a reimplementation gets subtly wrong: integer versus float subtypes, what `#`
//! reports, when a metamethod fires, and which errors are catchable.

use oslo_luavm::Engine;

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
