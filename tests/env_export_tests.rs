//! `oslo.env.set` and the export flag it never had.
//!
//! # What this exists to catch
//!
//! The binding hardcoded `true`, so **a directory environment could only ever create exported
//! variables**. A project name, a chosen profile, a computed cache key — all of it reached every
//! child that project ever spawned, which is how a `$DATABASE_URL`-shaped value ends up in an
//! editor's environment and then in its crash report.
//!
//! # The trap in the obvious fix
//!
//! Passing `false` through to `set_var` does nothing. It **ORs** the flag it is given with the one
//! the variable already carries (`env::scope::set_var`), so a name that is exported stays exported
//! and the caller is told it succeeded. The only way down is to clear the entry and write it again
//! — the same dance `Direnv::unload` does — and that in turn needs a readonly guard, because
//! `unset_var` does not check and `set_var` does: without it the pair deletes a readonly variable
//! and then declines to put it back.
//!
//! Both halves are pinned below.

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
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The default is unchanged, so nothing written against the old spelling moves.
#[test]
fn set_still_exports_by_default() {
    let said = lua(r#"
oslo.env.set("A", "yes")
print("child=[" .. oslo.run{"sh","-c","printf %s \"$A\"", capture=true}.out .. "]")
"#);
    assert!(said.contains("child=[yes]"), "{said}");
}

/// **The point of the whole item**: a variable this shell can see and no child can.
#[test]
fn a_variable_can_be_shell_local() {
    let said = lua(r#"
oslo.env.set("A", "yes", { export = false })
print("shell=" .. tostring(oslo.env.get("A")))
print("child=[" .. oslo.run{"sh","-c","printf %s \"$A\"", capture=true}.out .. "]")
"#);
    assert!(
        said.contains("shell=yes"),
        "the shell cannot see it\n{said}"
    );
    assert!(
        said.contains("child=[]"),
        "a child saw a shell-local variable\n{said}"
    );
}

/// **The trap.** `set_var` ORs the flag, so demoting a name that is *already* exported is the case
/// a naive implementation gets silently wrong — it reports success and changes nothing.
#[test]
fn export_false_demotes_a_name_that_was_already_exported() {
    let said = lua(r#"
oslo.env.set("A", "1")
print("before=[" .. oslo.run{"sh","-c","printf %s \"$A\"", capture=true}.out .. "]")
oslo.env.set("A", "2", { export = false })
print("after=[" .. oslo.run{"sh","-c","printf %s \"$A\"", capture=true}.out .. "]")
print("shell=" .. tostring(oslo.env.get("A")))
"#);
    assert!(said.contains("before=[1]"), "{said}");
    assert!(
        said.contains("after=[]"),
        "the demotion was a no-op — set_var's OR won\n{said}"
    );
    assert!(
        said.contains("shell=2"),
        "the new value was lost in the demotion\n{said}"
    );
}

/// A readonly name is refused *before* the unset, or the pair would delete it and fail to restore
/// it. The variable must still be there afterwards, with its old value.
#[test]
fn demoting_a_readonly_name_leaves_it_alone() {
    let said = lua(r#"
oslo.proc.exec("readonly A=keep")
local ok, why = oslo.env.set("A", "changed", { export = false })
print("refused=" .. tostring(ok == nil))
print("why=" .. tostring(why))
print("still=" .. tostring(oslo.env.get("A")))
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(
        said.contains("read only"),
        "the reason is not named\n{said}"
    );
    assert!(
        said.contains("still=keep"),
        "a readonly variable was destroyed\n{said}"
    );
}

/// The write result is answered rather than discarded, so a config stops believing in writes that
/// did not happen.
#[test]
fn a_refused_write_is_reported() {
    let said = lua(r#"
oslo.proc.exec("readonly A=keep")
local ok, why = oslo.env.set("A", "changed")
print("ok=" .. tostring(ok))
print("why=" .. tostring(why))
"#);
    assert!(
        said.contains("ok=nil"),
        "a refused write reported success\n{said}"
    );
    assert!(said.contains("why="), "{said}");
}

#[test]
fn all_lists_exported_by_default_and_everything_when_asked() {
    let said = lua(r#"
oslo.env.set("SHOWN", "1")
oslo.env.set("HIDDEN", "1", { export = false })
print("default_shown=" .. tostring(oslo.env.all()["SHOWN"] ~= nil))
print("default_hidden=" .. tostring(oslo.env.all()["HIDDEN"] ~= nil))
print("asked_hidden=" .. tostring(oslo.env.all{ exported = false }["HIDDEN"] ~= nil))
"#);
    assert!(said.contains("default_shown=true"), "{said}");
    assert!(
        said.contains("default_hidden=false"),
        "the default listed a shell-local name\n{said}"
    );
    assert!(said.contains("asked_hidden=true"), "{said}");
}

/// `snapshot` carries the third fact `all` cannot: which names are exported.
#[test]
fn a_snapshot_carries_the_export_flag() {
    let said = lua(r#"
oslo.env.set("SHOWN", "a")
oslo.env.set("HIDDEN", "b", { export = false })
for _, v in ipairs(oslo.env.snapshot()) do
  if v.name == "SHOWN" or v.name == "HIDDEN" then
    print(v.name .. "=" .. v.value .. "/" .. tostring(v.exported))
  end
end
"#);
    assert!(said.contains("SHOWN=a/true"), "{said}");
    assert!(said.contains("HIDDEN=b/false"), "{said}");
}
