//! `oslo.nix`'s named helpers, which are Lua and are therefore tested as Lua.
//!
//! **No nix is involved.** Every helper goes through `oslo.nix.run`, so a case replaces that one
//! function with a Lua stub and asserts on what the helper made of the document. That covers the
//! part that can be wrong — reading the lock, skipping the decoder's tag, ordering, the shape of
//! what comes back — without needing a flake, a store, or nix on the machine running CI.
//!
//! The invocation itself is covered against a fake `nix` binary in `oslo_shell::nix_shell::json`.

#![cfg(feature = "nix")]

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run `source` as Lua and answer stdout, failing loudly on a Lua error.
fn lua(source: &str) -> String {
    let dir = tempfile::tempdir().expect("temp dir");
    let script = dir.path().join("case.lua");
    std::fs::write(&script, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&script)
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A stub `oslo.nix.run` answering a document per query, written as Lua.
///
/// `__json` is on every table because that is what `oslo.json` puts there, and a helper walking a
/// decoded object has to step over it. Leaving it out of the fixtures would hide the one bug this
/// is most likely to have.
const STUB: &str = r#"
local DAY = 86400
local now = os.time()
oslo.nix.run = function(argv)
  local what = argv[1] .. " " .. argv[2]
  if what == "flake metadata" then
    return {
      __json = "object",
      description = "a test flake",
      dirtyRevision = "cafe-dirty",
      locks = { __json = "object", nodes = { __json = "object",
        root = { __json = "object" },
        nixpkgs = { __json = "object",
          locked = { __json = "object", type = "github", lastModified = now - 100 * DAY } },
        ancient = { __json = "object",
          locked = { __json = "object", type = "github", lastModified = now - 900 * DAY } },
      } },
    }
  elseif what == "flake show" then
    return { __json = "object",
      devShells = { __json = "object",
        ["x86_64-linux"] = { __json = "object",
          default = { __json = "object" }, tooling = { __json = "object" } },
        ["aarch64-linux"] = { __json = "object", default = { __json = "object" } },
      },
      packages = { __json = "object",
        ["x86_64-linux"] = { __json = "object",
          hello = { __json = "object", description = "the usual greeting" } },
      },
    }
  elseif what == "config show" then
    return { __json = "object",
      system = { __json = "object", value = "x86_64-linux" } }
  end
  return nil, "unexpected: " .. what
end
"#;

fn case(body: &str) -> String {
    lua(&format!("{STUB}\n{body}"))
}

#[test]
fn the_inputs_come_back_oldest_first_with_their_age_in_days() {
    let out = case(
        r#"
        for _, i in ipairs(oslo.nix.inputs()) do
          print(i.name, i.type, i.days)
        end
        "#,
    );
    assert_eq!(out, "ancient\tgithub\t900\nnixpkgs\tgithub\t100\n");
}

/// The root node is the flake itself and has nothing pinned; the decoder's tag is not an input.
#[test]
fn neither_the_root_node_nor_the_json_tag_is_reported_as_an_input() {
    let out = case(
        r#"
        local names = {}
        for _, i in ipairs(oslo.nix.inputs()) do names[#names + 1] = i.name end
        print(#names, table.concat(names, ","))
        "#,
    );
    assert_eq!(out, "2\tancient,nixpkgs\n");
}

#[test]
fn a_dirty_revision_is_reported_as_dirty() {
    assert_eq!(case("print(oslo.nix.dirty())"), "true\n");
    // A flake with nothing uncommitted has no `dirtyRevision` at all.
    let clean = case(
        r#"
        local was = oslo.nix.run
        oslo.nix.run = function(argv)
          local doc = was(argv)
          doc.dirtyRevision = nil
          return doc
        end
        print(oslo.nix.dirty())
        "#,
    );
    assert_eq!(clean, "false\n");
}

#[test]
fn the_shells_are_the_ones_this_machine_can_enter() {
    // Both systems are in the document; only the one nix builds for here is answered.
    let out = case(r#"print(table.concat(oslo.nix.shells(), ","))"#);
    assert_eq!(out, "default,tooling\n");
}

#[test]
fn a_system_may_be_named_instead_of_asked_for() {
    let out = case(r#"print(table.concat(oslo.nix.shells{system = "aarch64-linux"}, ","))"#);
    assert_eq!(out, "default\n");
}

#[test]
fn the_documents_own_fields_are_reachable_as_written() {
    let out = case(r#"print(oslo.nix.metadata().description, oslo.nix.system())"#);
    assert_eq!(out, "a test flake\tx86_64-linux\n");
}

/// A helper hands back `nil, message` the way every fallible call in `oslo.*` does.
#[test]
fn a_failure_travels_out_as_nil_and_a_message() {
    let out = lua(r#"
        oslo.nix.run = function() return nil, "error: no flake here" end
        local shells, err = oslo.nix.shells()
        print(shells, err)
        "#);
    assert_eq!(out, "nil\terror: no flake here\n");
}

/// `prior[1]` is the command itself, so the subcommand is `prior[2]`.
#[test]
fn the_outputs_a_subcommand_can_take_are_the_ones_offered() {
    let out = case(
        r#"
        local function show(sub, current)
          local found = oslo.nix.complete({ "nix", sub }, current)
          local names = {}
          for _, c in ipairs(found or {}) do names[#names + 1] = c[1] end
          print(sub, current, #names == 0 and "-" or table.concat(names, ","))
        end
        show("develop", ".#")
        show("build", ".#")
        show("develop", ".#too")
        "#,
    );
    assert_eq!(
        out,
        "develop\t.#\t.#default,.#tooling\n\
         build\t.#\t.#hello\n\
         develop\t.#too\t.#tooling\n"
    );
}

#[test]
fn a_flag_and_an_unknown_subcommand_fall_through_to_oslo() {
    let out = case(
        r#"
        print(oslo.nix.complete({ "nix", "build" }, "--op"))
        print(oslo.nix.complete({ "nix", "flake" }, ".#"))
        "#,
    );
    assert_eq!(out, "nil\nnil\n");
}

/// Evaluating somebody else's flake on a keystroke is how a Tab becomes a 46-second wait.
#[test]
fn a_named_flake_is_not_evaluated_for_a_keystroke() {
    let out = case(r#"print(oslo.nix.complete({ "nix", "build" }, "nixpkgs#hel"))"#);
    assert_eq!(out, "nil\n");
}

#[test]
fn a_candidate_carries_its_description_when_the_flake_gives_one() {
    let out = case(
        r#"
        local found = oslo.nix.complete({ "nix", "build" }, ".#")
        print(found[1][1], found[1][2])
        "#,
    );
    assert_eq!(out, ".#hello\tthe usual greeting\n");
}

/// The point of writing them in Lua: a config replaces one, and everything else keeps working.
#[test]
fn a_helper_can_be_replaced_by_a_config() {
    let out = case(
        r#"
        oslo.nix.dirty = function() return false end
        print(oslo.nix.dirty(), oslo.nix.metadata().description)
        "#,
    );
    assert_eq!(out, "false\ta test flake\n");
}
