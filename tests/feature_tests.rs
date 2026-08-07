//! `oslo.feature` — turning parts of the shell off and on while it runs.
//!
//! Two halves, tested two ways.
//!
//! **Builtin gating** is tested by spawning the binary, because it has to be observed the way a
//! user observes it — `type rm` naming a path — and because the bitset is process-global, so a
//! spawned shell cannot be disturbed by a sibling test.
//!
//! **Predicates** are tested in process against `feature::decide`, which is the function the read
//! loop calls from `environments::arrive`. Each test claims a *different* feature, so the shared
//! bitset is never contended even with tests running as threads.

mod common;

use common::oslo_bin;
use oslo::env::Environment;
use oslo::feature::{self, at};
use oslo::lua::LuaEngine;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

/// Run a Lua program through the real binary, in a scratch directory.
fn lua(program: &str) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, program).expect("write");
    let output: Output = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_owned(),
        String::from_utf8_lossy(&output.stderr)
            .trim_end()
            .to_owned(),
    )
}

/// An engine with the bindings installed, kept alive by the caller.
fn engine() -> (LuaEngine, Arc<Mutex<Environment>>) {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init");
    lua.setup_bindings(Arc::clone(&env)).expect("bindings");
    (lua, env)
}

// ------------------------------------------------------------------ the builtin gate

/// **The case the whole thing exists for.** oslo's `direnv` builtin cannot read an `.envrc`; the
/// real one can. Turning the feature off has to hand the name back to `$PATH`.
#[test]
fn a_disabled_feature_hands_its_builtin_back_to_the_path() {
    let (out, err) = lua(r#"
        oslo.run{"type", "rm"}
        oslo.feature.set("rm", false)
        oslo.run{"type", "rm"}
        "#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.first(),
        Some(&"rm is a shell builtin"),
        "stderr: {err}"
    );
    assert!(
        lines.get(1).is_some_and(|l| l.contains("/rm")),
        "rm should have become a path, got {lines:?}; stderr: {err}"
    );
}

/// Turning it back on restores the builtin. Nothing was removed from the registry — the bit is a
/// mask — so there is no re-registration to get wrong.
#[test]
fn turning_a_feature_back_on_restores_its_builtin() {
    let (out, err) = lua(r#"
        oslo.feature.set("rm", false)
        oslo.feature.set("rm", true)
        oslo.run{"type", "rm"}
        "#);
    assert_eq!(out, "rm is a shell builtin", "stderr: {err}");
}

/// A name that is not a feature is an error, not a shrug.
///
/// A config that turns off `direnvv` and is quietly obeyed looks like it works and does nothing —
/// which is indistinguishable from every other reason a config might not be taking effect.
#[test]
fn an_unknown_feature_name_is_refused_by_name() {
    let (out, err) = lua(r#"
        local ok, e = pcall(function() oslo.feature.set("direnvv", false) end)
        print(tostring(ok))
        print(e)
        "#);
    assert!(out.starts_with("false"), "expected a raise; stderr: {err}");
    assert!(
        out.contains("no feature called") && out.contains("direnv"),
        "the message should name the mistake and list the real ones: {out:?}"
    );
}

/// `list` reports every feature and what it does, so a prompt can show them.
#[test]
fn every_feature_is_listed_with_its_state() {
    let (out, err) = lua(r#"
        for _, f in ipairs(oslo.feature.list()) do
          print(f.name .. " " .. tostring(f.on) .. " " .. (#f.about > 0 and "described" or "BARE"))
        end
        "#);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), feature::FEATURES.len(), "stderr: {err}");
    for line in &lines {
        assert!(line.ends_with(" true described"), "{line:?}");
    }
}

// ------------------------------------------------------------------ predicates

/// `when` is re-asked per directory, so walking out of a directory undoes what walking in did —
/// with nothing recorded and nothing restored.
///
/// Claims `notify`, which no other test here touches.
#[test]
fn a_predicate_is_recomputed_for_each_directory() {
    let (lua_engine, _env) = engine();
    lua_engine
        .eval_script(r#"oslo.feature.when("notify", function(dir) return dir ~= "/off" end)"#)
        .expect("register");

    oslo::lua::api::feature::decide(std::path::Path::new("/off"));
    assert!(!feature::on(at::NOTIFY), "the predicate said no");

    oslo::lua::api::feature::decide(std::path::Path::new("/elsewhere"));
    assert!(
        feature::on(at::NOTIFY),
        "walking out must undo it, with no restore step"
    );
}

/// A predicate that returns nothing has not answered, and must not be read as `false`.
///
/// Otherwise a handler with a missing `return` on one branch silently turns a feature off, which
/// is the failure that is hardest to attribute to the config that caused it.
///
/// Claims `marks`.
#[test]
fn a_predicate_that_answers_nothing_leaves_the_feature_alone() {
    let (lua_engine, _env) = engine();
    feature::set(at::MARKS, false);
    lua_engine
        .eval_script(r#"oslo.feature.when("marks", function(dir) end)"#)
        .expect("register");

    oslo::lua::api::feature::decide(std::path::Path::new("/anywhere"));
    assert!(
        !feature::on(at::MARKS),
        "no answer must leave the bit as it was"
    );
    feature::set(at::MARKS, true);
}

/// A predicate that raises is reported and skipped, and does not decide the feature by accident.
///
/// Claims `finder`.
#[test]
fn a_predicate_that_raises_does_not_change_anything() {
    let (lua_engine, _env) = engine();
    lua_engine
        .eval_script(r#"oslo.feature.when("finder", function(dir) error("boom") end)"#)
        .expect("register");

    oslo::lua::api::feature::decide(std::path::Path::new("/anywhere"));
    assert!(
        feature::on(at::FINDER),
        "a broken predicate must not turn the feature off"
    );
}

/// `set` on a feature a predicate owns is refused rather than silently overwritten.
///
/// The write would appear to work and then be undone by the next `cd`, which is worse than being
/// told no.
#[test]
fn set_is_refused_where_a_predicate_decides() {
    let (out, err) = lua(r#"
        oslo.feature.when("suggest", function(dir) return true end)
        local ok, e = pcall(function() oslo.feature.set("suggest", false) end)
        print(tostring(ok))
        print(e)
        "#);
    assert!(out.starts_with("false"), "expected a raise; stderr: {err}");
    assert!(
        out.contains("oslo.feature.when"),
        "the message should point at the predicate: {out:?}"
    );
}

/// `when` needs a function. A string would be stored and then fail on every directory change,
/// which is a config error reported a long way from the line that caused it.
#[test]
fn when_refuses_anything_that_is_not_a_function() {
    let (out, err) = lua(r#"
        local ok, e = pcall(function() oslo.feature.when("vi", "yes") end)
        print(tostring(ok))
        print(e)
        "#);
    assert!(out.starts_with("false"), "expected a raise; stderr: {err}");
    assert!(out.contains("needs a function"), "{out:?}");
}

// ------------------------------------------------------------------ the mask

/// **A feature is a mask over configuration, never an assignment to it.**
///
/// A shell configured for emacs must not acquire vi mode because something enabled the `vi`
/// feature. This is what makes disable-and-restore work without recording the previous value.
#[test]
fn enabling_a_feature_does_not_turn_on_what_the_config_left_off() {
    let (out, err) = lua(r#"
        oslo.feature.set("vi", true)
        print(tostring(oslo.feature.get("vi")))
        "#);
    assert_eq!(out, "true", "stderr: {err}");
    // The configured value is the other half, and it is `false` by default — `oslo.vi.enabled`
    // ships off. The feature being on therefore cannot mean vi is on.
    assert!(
        !oslo::ui::vi::enabled(),
        "the feature bit alone must not enable vi; the config has not asked for it"
    );
}
