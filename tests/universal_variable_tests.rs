//! `oslo.env.universal` — a variable that outlives this shell.
//!
//! # Why these are integration tests and not unit tests
//!
//! `env::universal::isolate_for_tests` is `#[cfg(test)]` **inside `oslo-shell`**, so it does not
//! exist when `oslo-runtime` is the crate under test: a unit test there would resolve the store
//! through `$XDG_DATA_HOME` and write the **real** one — the person running the suite would find
//! their own universal variables edited. Spawning the binary with `HOME` and `XDG_DATA_HOME`
//! pointed at a tempdir is the only isolation available from outside that crate.
//!
//! # What they pin
//!
//! The write path is four steps, not one: the file, *this* shell, the applied record, and the
//! announcement. A binding that called `universal::set` alone would pass a test that only reopened
//! the file, and fail the person who ran the command and saw nothing happen. The first two tests
//! are that pair.

mod common;

use common::oslo_bin;
use std::path::Path;
use std::process::{Command, Stdio};

/// Run a chunk with a store of its own, and answer everything it printed.
fn lua_in(dir: &Path, source: &str) -> String {
    let file = dir.join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir)
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn lua(source: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    lua_in(dir.path(), source)
}

/// **Step two of four.** Setting one is visible to the shell that set it, immediately.
#[test]
fn a_universal_is_applied_to_this_shell_at_once() {
    let said = lua(r#"
oslo.env.universal_set("THEME", "dark")
print("here=" .. tostring(oslo.env.get("THEME")))
"#);
    assert!(
        said.contains("here=dark"),
        "written to the file but not to this shell\n{said}"
    );
}

/// **Step one of four**, and the point of the whole thing: a later shell sees it.
#[test]
fn a_universal_outlives_the_shell_that_set_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    lua_in(dir.path(), r#"oslo.env.universal_set("THEME", "dark")"#);
    let said = lua_in(
        dir.path(),
        r#"print("later=" .. tostring(oslo.env.universal("THEME").value))"#,
    );
    assert!(said.contains("later=dark"), "{said}");
}

#[test]
fn reading_one_that_is_not_there_is_nil() {
    let said = lua(r#"print("missing=" .. tostring(oslo.env.universal("NOPE")))"#);
    assert!(said.contains("missing=nil"), "{said}");
}

#[test]
fn reading_them_all_answers_a_table_keyed_by_name() {
    let said = lua(r#"
oslo.env.universal_set("A", "1")
oslo.env.universal_set("B", "2")
local all = oslo.env.universal()
print("a=" .. all.A.value .. " b=" .. all.B.value)
"#);
    assert!(said.contains("a=1 b=2"), "{said}");
}

/// **Unexported by default**, the opposite of `oslo.env.set`, because a universal is written once
/// and read for months.
#[test]
fn a_universal_does_not_reach_a_child_unless_asked() {
    let said = lua(r#"
oslo.env.universal_set("QUIET", "yes")
oslo.env.universal_set("LOUD", "yes", { export = true })
print("quiet=[" .. oslo.run{"sh","-c","printf %s \"$QUIET\"", capture=true}.out .. "]")
print("loud=[" .. oslo.run{"sh","-c","printf %s \"$LOUD\"", capture=true}.out .. "]")
"#);
    assert!(
        said.contains("quiet=[]"),
        "an unexported universal leaked\n{said}"
    );
    assert!(said.contains("loud=[yes]"), "{said}");
}

#[test]
fn erasing_removes_it_from_the_store_and_from_this_shell() {
    let said = lua(r#"
oslo.env.universal_set("GONE", "x")
oslo.env.universal_erase("GONE")
print("stored=" .. tostring(oslo.env.universal("GONE")))
print("here=" .. tostring(oslo.env.get("GONE")))
"#);
    assert!(said.contains("stored=nil"), "{said}");
    assert!(said.contains("here=nil"), "left set in this shell\n{said}");
}

#[test]
fn erasing_one_that_was_never_universal_answers_false() {
    let said = lua(r#"print("erased=" .. tostring(oslo.env.universal_erase("NEVER")))"#);
    assert!(said.contains("erased=false"), "{said}");
}

#[test]
fn an_invalid_name_is_refused_rather_than_written() {
    let said = lua(r#"
local ok, why = oslo.env.universal_set("not a name", "x")
print("refused=" .. tostring(ok == nil))
print("says=" .. tostring(why ~= nil))
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("says=true"), "{said}");
}

// ───────────────────────────────────────────── reading works where the rest of oslo.env does not

/// **The half that makes this worth having.** The store is a file, not the shell, so a registered
/// builtin can read it — while `oslo.env.get` beside it still raises.
#[test]
fn a_builtin_can_read_the_store_but_not_the_shell() {
    let said = lua(r#"
oslo.env.universal_set("THEME", "dark")
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  local ok, err = pcall(function() return oslo.env.universal("THEME").value end)
  print("read=" .. tostring(ok and "works" or "refused"))
  if ok then print("value=" .. oslo.env.universal("THEME").value) end
  local locked = pcall(function() return oslo.env.get("THEME") end)
  print("env_get=" .. tostring(locked and "works" or "refused"))
end }
oslo.proc.exec("probe")
"#);
    assert!(said.contains("read=works"), "reading took the lock\n{said}");
    assert!(said.contains("value=dark"), "{said}");
    assert!(
        said.contains("env_get=refused"),
        "the lock boundary moved\n{said}"
    );
}

/// And the write half, which a builtin reaches through the effects table instead.
#[test]
fn a_builtin_persists_through_the_effects_table() {
    let dir = tempfile::tempdir().expect("tempdir");
    let said = lua_in(
        dir.path(),
        r#"
oslo.register_builtin{ name = "remember", run = function(argv, shell)
  return { universal = { THEME = "dark", EDITOR = { value = "hx", export = true } } }
end }
oslo.proc.exec("remember")
print("here=" .. tostring(oslo.env.get("THEME")))
"#,
    );
    assert!(
        said.contains("here=dark"),
        "the effect did not reach this shell\n{said}"
    );
    let later = lua_in(
        dir.path(),
        r#"
print("theme=" .. tostring(oslo.env.universal("THEME").value))
print("exported=" .. tostring(oslo.env.universal("EDITOR").exported))
"#,
    );
    assert!(later.contains("theme=dark"), "it did not persist\n{later}");
    assert!(later.contains("exported=true"), "{later}");
}

#[test]
fn a_builtin_can_erase_with_false() {
    let dir = tempfile::tempdir().expect("tempdir");
    lua_in(dir.path(), r#"oslo.env.universal_set("GONE", "x")"#);
    lua_in(
        dir.path(),
        r#"
oslo.register_builtin{ name = "forget", run = function() return { universal = { GONE = false } } end }
oslo.proc.exec("forget")
"#,
    );
    let later = lua_in(
        dir.path(),
        r#"print("stored=" .. tostring(oslo.env.universal("GONE")))"#,
    );
    assert!(later.contains("stored=nil"), "{later}");
}
