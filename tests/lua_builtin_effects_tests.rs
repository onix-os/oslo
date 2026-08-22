//! A registered builtin, handed the shell and returning what it wants changed.
//!
//! # What this exists to catch
//!
//! `register_builtin` bound the environment and threw it away:
//!
//! ```ignore
//! guard.register_dynamic_builtin(&declared.name, move |_env, args| { … })
//! ```
//!
//! So a builtin could not see `$?` **by any route** — the `oslo.*` call raised *shell state is
//! busy*, and the Lua-global spelling `try_lock`ed and answered nil — and a plain `X = "1"` was a
//! `try_lock` that dropped the write with no message at all. The registry clones the callback out
//! precisely so the closure can hold a live `&mut Environment`; its own comment says so. The
//! capability was designed in and never used.
//!
//! The first test here is the one the whole change is for.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

fn lua(source: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .env_remove("ENV")
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// **The headline.** A builtin can see the status of the command before it.
#[test]
fn a_builtin_can_see_dollar_question() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  print("status=" .. tostring(shell.status) .. " ok=" .. tostring(shell.ok))
end }
oslo.proc.exec("false")
oslo.proc.exec("probe")
"#);
    assert!(
        said.contains("status=1 ok=false"),
        "a builtin still cannot see $?\n{said}"
    );
}

#[test]
fn the_cheap_record_is_there_without_asking() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  print("name=" .. shell.name)
  print("has_cwd=" .. tostring(shell.cwd ~= nil))
  print("pid_positive=" .. tostring(shell.pid > 0))
  print("flags_is_string=" .. tostring(type(shell.flags) == "string"))
  print("argc=" .. tostring(shell.argc))
end }
oslo.proc.exec("probe")
"#);
    for want in [
        "name=probe",
        "has_cwd=true",
        "pid_positive=true",
        "flags_is_string=true",
    ] {
        assert!(said.contains(want), "{want} missing\n{said}");
    }
}

/// `argv[1]` is still the builtin's own name — the one contract deliberately left asymmetrical with
/// the record's 1-based lists.
#[test]
fn argv_is_unchanged_and_arrives_beside_the_record() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  print("argv=" .. table.concat(argv, ","))
  print("both=" .. tostring(shell ~= nil))
end }
oslo.proc.exec("probe one two")
"#);
    assert!(said.contains("argv=probe,one,two"), "{said}");
    assert!(said.contains("both=true"), "{said}");
}

/// The second thing a builtin could not do: see a variable no child process can.
#[test]
fn wants_vars_shows_an_unexported_variable() {
    let said = lua(r#"
oslo.env.set("HIDDEN", "secret", { export = false })
oslo.register_builtin{ name = "probe", wants = { "vars" }, run = function(argv, shell)
  print("hidden=" .. tostring(shell.vars.HIDDEN))
end }
oslo.proc.exec("probe")
"#);
    assert!(said.contains("hidden=secret"), "{said}");
}

/// **The reason `wants` is safe to be opt-in.** A field not asked for raises and says how to fix it,
/// rather than being `nil` and turning into a silent nothing.
#[test]
fn a_field_that_was_not_asked_for_says_so() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  local ok, err = pcall(function() return shell.aliases end)
  print("refused=" .. tostring(not ok))
  print("names_the_fix=" .. tostring(tostring(err):find("wants") ~= nil))
end }
oslo.proc.exec("probe")
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("names_the_fix=true"), "{said}");
}

#[test]
fn an_unknown_wants_name_fails_at_registration() {
    let said = lua(r#"
local ok, err = pcall(function()
  oslo.register_builtin{ name = "probe", wants = { "variables" }, run = function() end }
end)
print("refused=" .. tostring(not ok))
print("lists_them=" .. tostring(tostring(err):find("aliases") ~= nil))
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("lists_them=true"), "{said}");
}

// ─────────────────────────────────────────────────────────────── effects

#[test]
fn set_is_shell_local_and_export_reaches_a_child() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function(argv, shell)
  return { set = { LOCAL = "a" }, export = { SHARED = "b" } }
end }
oslo.proc.exec("probe")
print("shell_sees_local=" .. tostring(oslo.env.get("LOCAL")))
print("child_local=[" .. oslo.run{"sh","-c","printf %s \"$LOCAL\"", capture=true}.out .. "]")
print("child_shared=[" .. oslo.run{"sh","-c","printf %s \"$SHARED\"", capture=true}.out .. "]")
"#);
    assert!(said.contains("shell_sees_local=a"), "{said}");
    assert!(
        said.contains("child_local=[]"),
        "set must not export\n{said}"
    );
    assert!(said.contains("child_shared=[b]"), "{said}");
}

/// **`unset` before `set`**, or a name written in both is silently discarded.
#[test]
fn the_order_lets_a_name_be_unset_and_set_again() {
    let said = lua(r#"
oslo.env.set("A", "old")
oslo.register_builtin{ name = "probe", run = function()
  return { unset = { "A" }, set = { A = "new" } }
end }
oslo.proc.exec("probe")
print("A=" .. tostring(oslo.env.get("A")))
"#);
    assert!(said.contains("A=new"), "unset ran after set\n{said}");
}

#[test]
fn an_alias_can_be_added_and_removed() {
    let said = lua(r#"
oslo.register_builtin{ name = "adder", run = function() return { alias = { zz = "echo hi" } } end }
oslo.register_builtin{ name = "remover", run = function() return { alias = { zz = false } } end }
oslo.proc.exec("adder")
print("after_add=" .. tostring(oslo.env.alias("zz")))
oslo.proc.exec("remover")
print("after_remove=" .. tostring(oslo.env.alias("zz")))
"#);
    assert!(said.contains("after_add=echo hi"), "{said}");
    assert!(said.contains("after_remove=nil"), "{said}");
}

/// `cd` goes through the builtin, so `PWD` and `pwd` agree — which is why it is not a
/// `set_current_dir`.
#[test]
fn cd_moves_the_shell_and_pwd_agrees() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function() return { cd = "/tmp" } end }
oslo.proc.exec("probe")
print("pwd=" .. tostring(oslo.env.get("PWD")))
print("cwd=" .. oslo.fs.cwd())
"#);
    assert!(said.contains("pwd=/tmp"), "{said}");
    assert!(
        said.contains("cwd=/tmp"),
        "PWD and the real cwd disagree\n{said}"
    );
}

/// Every scalar spelling still works — the regression guard on `status_from_lua` surviving its move
/// out of `engine.rs`.
#[test]
fn the_scalar_returns_all_still_mean_what_they_meant() {
    let said = lua(r#"
oslo.register_builtin{ name = "nothing", run = function() end }
oslo.register_builtin{ name = "yes", run = function() return true end }
oslo.register_builtin{ name = "no", run = function() return false end }
oslo.register_builtin{ name = "seven", run = function() return 7 end }
oslo.register_builtin{ name = "worded", run = function() return "3" end }
for _, name in ipairs({"nothing","yes","no","seven","worded"}) do
  oslo.proc.exec(name)
  print(name .. "=" .. tostring(oslo.proc.status()))
end
"#);
    for want in ["nothing=0", "yes=0", "no=1", "seven=7", "worded=3"] {
        assert!(said.contains(want), "{want}\n{said}");
    }
}

/// **All-or-nothing on a malformed return.** A misspelt key raises, and the well-formed sibling
/// beside it must not have been applied — otherwise a typo leaves the shell half-changed.
#[test]
fn an_unknown_effect_applies_nothing_at_all() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function()
  return { set = { GOOD = "1" }, setenv = { BAD = "2" } }
end }
oslo.proc.exec("probe")
print("status=" .. tostring(oslo.proc.status()))
print("good=" .. tostring(oslo.env.get("GOOD")))
"#);
    assert!(said.contains("status=1"), "{said}");
    assert!(
        said.contains("good=nil"),
        "a sibling effect was applied despite the refusal\n{said}"
    );
    assert!(said.contains("setenv"), "the bad key is not named\n{said}");
}

/// An explicit `status` is the script's word, even when an effect was refused.
#[test]
fn an_explicit_status_wins_over_a_refused_effect() {
    let said = lua(r#"
oslo.proc.exec("readonly RO=keep")
oslo.register_builtin{ name = "insists", run = function()
  return { set = { RO = "changed" }, status = 0 }
end }
oslo.register_builtin{ name = "quiet", run = function()
  return { set = { RO = "changed" } }
end }
oslo.proc.exec("insists")
print("insists=" .. tostring(oslo.proc.status()))
oslo.proc.exec("quiet")
print("quiet=" .. tostring(oslo.proc.status()))
"#);
    assert!(
        said.contains("insists=0"),
        "an explicit status was ignored\n{said}"
    );
    assert!(
        said.contains("quiet=1"),
        "a silently refused effect reported success\n{said}"
    );
}

/// An empty table means the same as returning nothing, so a field can be added later without an
/// existing handler's answer suddenly meaning more.
#[test]
fn an_empty_table_is_success_and_no_effect() {
    let said = lua(r#"
oslo.register_builtin{ name = "probe", run = function() return {} end }
oslo.proc.exec("probe")
print("status=" .. tostring(oslo.proc.status()))
"#);
    assert!(said.contains("status=0"), "{said}");
}
