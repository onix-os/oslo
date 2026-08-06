//! Every settings path the README documents must be one a config can actually assign to.
//!
//! # The bug this exists to catch
//!
//! A setting arrives in two steps, and only one of them was ever checked. The Rust side reads the
//! table back in `interactive::settings::from_lua`; the *Lua* side has to be able to reach it
//! first, and `oslo.misc.welcome = false` is an ordinary Lua assignment — index `oslo` by `misc`,
//! then set a field. If `oslo.misc` does not exist, indexing `nil` raises and **the whole config
//! file dies at that line**, silently taking every setting after it with it.
//!
//! So a namespace has to be pre-created as an empty table in `lua::api::install` before any config
//! can write into it. Nothing enforced that. Forgetting it produced no compile error, no warning
//! and no failing test — only a config that appeared to be ignored.
//!
//! It has happened twice. `vi`, `notify` and `dirs` were missing once, so `oslo.vi.enabled = false`
//! — the spelling the README documents — died while `oslo.vi = { enabled = false }` worked. Then
//! `oslo.builtin.rm.to_tmp` hit the same wall, and needed *two* levels because the path indexes
//! twice.
//!
//! The README is the right source of truth because it is the thing a user copies from. If a line
//! is documented, it has to work.

use oslo::env::Environment;
use oslo::lua::engine::LuaEngine;
use std::sync::{Arc, Mutex};

/// A documented assignment: the table it writes into, and the line it came from.
struct Documented {
    /// Everything before the last component — the part that must already be a table.
    parent: String,
    line: String,
}

/// Every `oslo.….… = …` line in the README, as the table each one needs to exist.
///
/// Hand-parsed rather than by regex: the shape is `oslo` followed by `.name` or `["name"]`
/// segments and then an `=`, and anything that does not look exactly like that is not a settings
/// assignment and is skipped rather than guessed at.
fn documented_settings(readme: &str) -> Vec<Documented> {
    let mut found = Vec::new();
    for raw in readme.lines() {
        let line = raw.trim();
        // A commented-out example is documentation of something *not* to run.
        if !line.starts_with("oslo.") || line.starts_with("--") {
            continue;
        }
        let Some(equals) = assignment_at(line) else {
            continue;
        };
        let target = line[..equals].trim_end();
        let Some(parent) = parent_of(target) else {
            continue;
        };
        // `oslo.x = …` needs nothing but `oslo`, which always exists.
        if parent == "oslo" {
            continue;
        }
        found.push(Documented {
            parent,
            line: line.to_string(),
        });
    }
    found
}

/// The index of the `=` that makes this an assignment, if it is one.
///
/// `==` is a comparison and `>=`/`~=` are not assignments either, so a line containing one is not
/// a documented setting. A `=` inside the *value* cannot be reached, because the first one wins.
fn assignment_at(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let at = line.find('=')?;
    if bytes.get(at + 1) == Some(&b'=') {
        return None;
    }
    if matches!(
        bytes.get(at.checked_sub(1)?),
        Some(b'=' | b'<' | b'>' | b'~')
    ) {
        return None;
    }
    Some(at)
}

/// `oslo.builtin.rm.to_tmp` -> `oslo.builtin.rm`; `oslo.keys["alt-a"]` -> `oslo.keys`.
///
/// `None` when the target is not a plain path — a call, an index by a variable, anything this
/// should not be guessing about.
fn parent_of(target: &str) -> Option<String> {
    if let Some(bracket) = target.rfind('[') {
        // Only a literal key. `oslo.keys[name]` depends on a variable and is not a fixed path.
        if !target[bracket..].contains('"') && !target[bracket..].contains('\'') {
            return None;
        }
        return Some(target[..bracket].trim_end().to_string());
    }
    let dot = target.rfind('.')?;
    let parent = &target[..dot];
    let ok = parent
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    ok.then(|| parent.to_string())
}

fn engine() -> LuaEngine {
    let lua = LuaEngine::new().expect("an interpreter");
    lua.setup_bindings(Arc::new(Mutex::new(Environment::new())))
        .expect("the oslo table");
    lua
}

/// **Every documented settings path is assignable.**
///
/// Asserted on the *parent* table rather than by running the assignment, so the test says nothing
/// about what a value may be — only that a config writing the documented line will not die on the
/// way to it.
#[test]
fn every_documented_setting_can_be_assigned() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md"))
        .expect("the README is beside Cargo.toml");
    let documented = documented_settings(&readme);

    assert!(
        documented.len() > 10,
        "only {} settings found — the parser has stopped matching the README",
        documented.len()
    );

    let lua = engine();
    let mut broken = Vec::new();
    for setting in &documented {
        // `type()` rather than the assignment itself: if any link in the chain is nil this raises,
        // which is exactly the failure a config would hit, and it needs no value to be valid.
        let probe = format!("__probe = type({})", setting.parent);
        let reached = lua.eval_script(&probe).is_ok()
            && lua.eval_script("__ok = __probe == 'table'").is_ok()
            && lua
                .eval_script("if not __ok then error('not a table') end")
                .is_ok();
        if !reached {
            broken.push(format!(
                "  {}\n    needs `{}`",
                setting.line, setting.parent
            ));
        }
    }

    assert!(
        broken.is_empty(),
        "these README lines would die with \"attempt to index a nil value\":\n{}\n\n\
         Add the missing name to the pre-created table list in `src/lua/api/mod.rs::install`.",
        broken.join("\n")
    );
}

/// The parser itself, because a test that quietly matches nothing passes for ever.
#[test]
fn the_readme_parser_finds_what_it_should_and_skips_what_it_should_not() {
    let sample = "\
oslo.misc.welcome       = true        -- a comment
oslo.builtin.rm.to_tmp  = false
oslo.keys[\"alt-a\"] = \"accept-suggestion\"
oslo.abbr.gco = \"git checkout\"
oslo.version = 1
-- oslo.commented.out = true
oslo.on.key(function(k) end)
if oslo.a.b == oslo.c.d then end
oslo.keys[name] = fn
";
    let found = documented_settings(sample);
    let parents: Vec<&str> = found.iter().map(|d| d.parent.as_str()).collect();
    assert_eq!(
        parents,
        vec!["oslo.misc", "oslo.builtin.rm", "oslo.keys", "oslo.abbr"],
        "a nested path keeps every level, and `oslo.version` needs only `oslo`"
    );

    // Each of the four skips is a different reason, and each has been a real line in the README.
    for skipped in [
        "-- oslo.commented.out = true",
        "oslo.on.key(function(k) end)",
        "if oslo.a.b == oslo.c.d then end",
        "oslo.keys[name] = fn",
    ] {
        assert!(
            documented_settings(skipped).is_empty(),
            "{skipped:?} is not a documented setting"
        );
    }
}

/// The test has to be able to *fail*. A name that was never pre-created must be reported, or the
/// green tick above means only that the parser found nothing wrong with itself.
#[test]
fn a_namespace_that_was_never_created_is_caught() {
    let lua = engine();
    assert!(
        lua.eval_script("__probe = type(oslo.definitely_not_a_namespace.field)")
            .is_err(),
        "indexing an absent namespace must raise — this is the failure being guarded against"
    );
    assert!(
        lua.eval_script("__probe = type(oslo.builtin.rm)").is_ok(),
        "and a real one must not"
    );
}
