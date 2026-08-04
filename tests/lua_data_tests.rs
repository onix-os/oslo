//! `oslo.json`, `oslo.re`, and the module system.
//!
//! These are the three things a Lua script cannot get any other way inside oslo: a C module
//! cannot be loaded into a static binary, so `lua-cjson` and every regex binding are unavailable,
//! and `require` had to be written rather than inherited.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run a Lua chunk in a fresh directory and return its stdout, trimmed.
#[track_caller]
fn lua(script: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    run_in(dir.path(), script)
}

#[track_caller]
fn run_in(dir: &std::path::Path, script: &str) -> String {
    let path = dir.join("case.lua");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir)
        .env("HOME", dir)
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

#[test]
fn json_decodes_into_ordinary_lua_values() {
    let out = lua(r#"
        local doc = oslo.json.decode('{"name":"oslo","count":3,"tags":["a","b"],"on":true}')
        print(doc.name, doc.count, doc.on)
        print(#doc.tags, doc.tags[1], doc.tags[2])
        -- An integral JSON number is a Lua integer, so it prints as 3 and can index a table.
        print(math.type(doc.count))
    "#);
    assert_eq!(out, "oslo\t3\ttrue\n2\ta\tb\ninteger");
}

/// JSON `null` must not decode to `nil`.
///
/// A nil in a table is indistinguishable from an absent key, so a null field would silently
/// vanish — and a script could not tell "not present" from "present and null".
#[test]
fn null_survives_decoding() {
    let out = lua(r#"
        local doc = oslo.json.decode('{"here":null}')
        print(doc.here, doc.missing)
    "#);
    assert_eq!(out, "false\tnil");
}

#[test]
fn encode_chooses_between_an_array_and_an_object() {
    let out = lua(r#"
        print(oslo.json.encode({1, 2, 3}))
        print(oslo.json.encode({a = 1}))
        -- An empty Lua table could be either; a list is the common case in a shell script.
        print(oslo.json.encode({}))
    "#);
    assert_eq!(out, "[1,2,3]\n{\"a\":1}\n[]");
}

/// A document that goes through Lua and back must keep its shape, empty containers included.
#[test]
fn decoding_marks_what_it_decoded_so_encoding_can_restore_it() {
    let out = lua(r#"
        for _, doc in ipairs({'{"a":{}}', '{"a":[]}', '[]', '{}'}) do
            print(oslo.json.encode(oslo.json.decode(doc)))
        end
    "#);
    assert_eq!(out, "{\"a\":{}}\n{\"a\":[]}\n[]\n{}");
}

#[test]
fn a_malformed_document_answers_rather_than_raising() {
    let out = lua(r#"
        local doc, err = oslo.json.decode("{not json")
        print(doc, err ~= nil)
    "#);
    assert_eq!(out, "nil\ttrue");
}

#[test]
fn a_table_containing_itself_is_reported_not_recursed_into() {
    let out = lua(r#"
        local t = {}
        t.self = t
        local ok, err = pcall(oslo.json.encode, t)
        print(ok, err:find("itself") ~= nil)
    "#);
    assert_eq!(out, "false\ttrue");
}

#[test]
fn re_matches_with_real_regular_expressions() {
    let out = lua(r#"
        -- Alternation, which Lua patterns simply do not have.
        local m = oslo.re.match("version 2.1", "(%d+)%.(%d+)")
        print(m == nil)
        m = oslo.re.match("version 2.1", "(\\d+)\\.(\\d+)")
        print(m.match, m.groups[1], m.groups[2], m.start, m.stop)
        print(oslo.re.test("cat", "^(cat|dog)$"), oslo.re.test("cow", "^(cat|dog)$"))
    "#);
    // Lua's `%d` is not a regex escape, so the first pattern matches nothing here.
    assert_eq!(out, "true\n2.1\t2\t1\t9\t11\ntrue\tfalse");
}

#[test]
fn a_group_that_did_not_participate_is_false_not_a_hole() {
    // nil in a sequence is a hole, so `#groups` would stop counting at the first optional group
    // that missed.
    let out = lua(r#"
        local m = oslo.re.match("a", "(a)(b)?")
        print(#m.groups, m.groups[1], m.groups[2])
    "#);
    assert_eq!(out, "2\ta\tfalse");
}

#[test]
fn re_covers_find_all_replace_split_and_quote() {
    let out = lua(r#"
        local all = oslo.re.find_all("a1 b2 c3", "([a-z])(\\d)")
        print(#all, all[2].groups[1] .. all[2].groups[2])
        print(oslo.re.replace("a-b-c", "-", "+"))
        print(oslo.re.replace("a-b-c", "-", "+", 1))
        print(table.concat(oslo.re.split("a, b,c", ",\\s*"), "|"))
        -- A quoted string matches itself and nothing else.
        print(oslo.re.test("a.c", oslo.re.quote("a.c")), oslo.re.test("abc", oslo.re.quote("a.c")))
    "#);
    assert_eq!(out, "3\tb2\na+b+c\na+b-c\na|b|c\ntrue\tfalse");
}

#[test]
fn an_invalid_pattern_is_a_mistake_in_the_script() {
    let out = lua(r#"
        local ok, err = pcall(oslo.re.test, "x", "(unclosed")
        print(ok, err:find("invalid pattern") ~= nil)
    "#);
    assert_eq!(out, "false\ttrue");
}

#[test]
fn require_loads_a_module_once_and_caches_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("lib")).unwrap();
    std::fs::write(
        dir.path().join("lib/counter.lua"),
        "COUNTED = (COUNTED or 0) + 1\nreturn {n = COUNTED}\n",
    )
    .unwrap();

    let out = run_in(
        dir.path(),
        r#"
        package.path = "./lib/?.lua"
        local a = require("counter")
        local b = require("counter")
        -- The same table, and the file ran once.
        print(a == b, a.n, COUNTED)
    "#,
    );
    assert_eq!(out, "true\t1\t1");
}

/// `package.path` must not search the working directory.
///
/// Stock Lua's default ends in `./?.lua`, so a `require` picks up whatever happens to be in the
/// directory you ran the command from. In a shell that is a script hijack.
#[test]
fn the_default_search_path_ignores_the_working_directory() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hijack.lua"), "print('LOADED')\nreturn 1\n").unwrap();

    let out = run_in(
        dir.path(),
        r#"
        print(package.path:find("%./%?") == nil)
        print(package.cpath == "")
        local ok, err = pcall(require, "hijack")
        print(ok, err:find("not found") ~= nil)
    "#,
    );
    assert_eq!(out, "true\ntrue\nfalse\ttrue");
}

#[test]
fn a_module_that_requires_itself_is_reported_as_a_loop() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("selfish.lua"),
        "require('selfish')\nreturn 1\n",
    )
    .unwrap();

    let out = run_in(
        dir.path(),
        r#"
        package.path = "./?.lua"
        local ok, err = pcall(require, "selfish")
        print(ok, err:find("loop") ~= nil)
    "#,
    );
    assert_eq!(out, "false\ttrue");
}

#[test]
fn a_module_that_fails_can_be_required_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("broken.lua"), "error('nope')\n").unwrap();

    // The in-progress marker must not outlive a failed load, or the second attempt would report
    // a loop rather than the real error.
    let out = run_in(
        dir.path(),
        r#"
        package.path = "./?.lua"
        local _, first = pcall(require, "broken")
        local _, second = pcall(require, "broken")
        print(first:find("nope") ~= nil, second:find("nope") ~= nil)
    "#,
    );
    assert_eq!(out, "true\ttrue");
}

#[test]
fn a_module_error_names_the_module_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("bad.lua"), "\n\nerror('inside')\n").unwrap();

    let out = run_in(
        dir.path(),
        r#"
        package.path = "./?.lua"
        local ok, err = pcall(require, "bad")
        print(err:find("bad.lua") ~= nil, err:find(":3:") ~= nil)
    "#,
    );
    assert_eq!(out, "true\ttrue");
}

#[test]
fn dofile_runs_the_file_every_time() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("once.lua"), "RAN = (RAN or 0) + 1\n").unwrap();

    let out = run_in(
        dir.path(),
        r#"
        dofile("once.lua")
        dofile("once.lua")
        print(RAN)
    "#,
    );
    assert_eq!(out, "2");
}

#[test]
fn load_compiles_a_string_into_a_callable_chunk() {
    let out = lua(r#"
        local f = load("local a, b = ... return a + b")
        print(f(2, 3), f(10, 1))
        -- A chunk that does not parse answers nil plus the reason.
        local bad, err = load("this is not lua(")
        print(bad, err ~= nil)
    "#);
    assert_eq!(out, "5\t11\nnil\ttrue");
}

#[test]
fn from_columns_reads_another_programs_table() {
    let out = lua(r#"
        local rows = oslo.from_columns("a 1 x\nb 2 y\n", {"name", "n", "tag"})
        print(#rows, rows[2].name, rows[2].n, rows[2].tag)
        -- With no header each line is a list of fields, split on runs of whitespace the way awk
        -- does: a leading indent produces no empty first field.
        local bare = oslo.from_columns("  a   b  \n")
        print(#bare, #bare[1], bare[1][1], bare[1][2])
    "#);
    assert_eq!(out, "2\tb\t2\ty\n1\t2\ta\tb");
}

/// A command that puts a whole command line in its last column — `ps` does — must not lose it.
#[test]
fn from_columns_keeps_a_trailing_column_whole() {
    let out = lua(r#"
        local rows = oslo.from_columns("123 oslo -c echo hi\n", {"pid", "command"})
        print(rows[1].pid, rows[1].command)
    "#);
    assert_eq!(out, "123\toslo -c echo hi");
}

#[test]
fn from_pairs_reads_the_key_value_output_half_the_system_speaks() {
    let out = lua(r#"
        local os_release = oslo.from_pairs('NAME="Alpine Linux"\nID=alpine\n')
        print(os_release.NAME, os_release.ID)
        -- A different separator, for the commands that use one.
        local colons = oslo.from_pairs("a: 1\nb: 2\n", ":")
        print(colons.a, colons.b)
    "#);
    assert_eq!(out, "Alpine Linux\talpine\n1\t2");
}

#[test]
fn from_lines_does_not_invent_a_trailing_empty_one() {
    let out = lua(r#"
        print(#oslo.from_lines("a\nb\n"), #oslo.from_lines("a\nb"), #oslo.from_lines(""))
    "#);
    assert_eq!(out, "2\t2\t0");
}

/// `oslo.from_json` is the same function as `oslo.json.decode`, not a second parser.
#[test]
fn from_json_is_the_json_decoder_under_the_family_name() {
    let out = lua(r#"
        print(oslo.from_json == oslo.json.decode, oslo.from_json('{"a":1}').a)
    "#);
    assert_eq!(out, "true\t1");
}

#[test]
fn proc_names_signals_rather_than_numbering_them() {
    let out = lua(r#"
        print(oslo.proc.alive(oslo.proc.pid()), oslo.proc.alive(2147483))
        print(oslo.proc.signal_number("TERM"), oslo.proc.signal_number("sigterm"))
        print(#oslo.proc.signals() > 10)
        -- A typo is refused, not quietly turned into SIGTERM.
        local ok, err = pcall(oslo.proc.kill, oslo.proc.pid(), "NOTASIGNAL")
        print(ok, err:find("no signal") ~= nil)
    "#);
    assert_eq!(out, "true\tfalse\n15\t15\ntrue\nfalse\ttrue");
}

#[test]
fn the_job_table_is_the_shells_own() {
    let out = lua(r#"
        print(#oslo.job.list())
        -- Naming a job that does not exist answers rather than raising.
        local ok, err = oslo.job.foreground(99)
        print(ok, err ~= nil)
    "#);
    assert_eq!(out, "0\nnil\ttrue");
}

#[test]
fn the_shell_can_describe_itself() {
    let out = lua(r#"
        print(oslo.version ~= nil, type(oslo.sys.host()), oslo.proc.pid() > 0, oslo.proc.ppid() > 0)
        -- A script run from a file is neither interactive nor a login shell.
        print(oslo.sys.interactive(), oslo.sys.login())
        oslo.proc.exec("false")
        print(oslo.proc.status())
        -- The script's own exit status is `$?` as it leaves it, so this puts it back to 0.
        oslo.proc.exec("true")
    "#);
    assert_eq!(out, "true\tstring\ttrue\ttrue\nfalse\tfalse\n1");
}

#[test]
fn options_are_shell_variables_under_a_namespace() {
    // One home for each setting, so `$OSLO_DEFAULT_MODE` set from either language means the same
    // thing — and so `oslo.opts.set` cannot reach an unrelated variable.
    let out = lua(r#"
        oslo.opts.set("default_mode", "lua")
        print(oslo.opts.get("default_mode"), oslo.env.get("OSLO_DEFAULT_MODE"))
        print(oslo.opts.get("never_set"))
        print(#oslo.opts.names() > 0)
    "#);
    assert_eq!(out, "lua\tlua\nnil\ntrue");
}

#[test]
fn a_required_module_receives_its_own_name_as_varargs() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("named.lua"), "return {who = ...}\n").unwrap();

    let out = run_in(
        dir.path(),
        r#"
        package.path = "./?.lua"
        print(require("named").who)
    "#,
    );
    assert_eq!(out, "named");
}
