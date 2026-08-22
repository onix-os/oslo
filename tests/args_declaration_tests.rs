//! `oslo.args` — the argument declaration a config can parse with.
//!
//! # Why this is a binding and not a new format
//!
//! The same argument list gets written three times in an oslo config and shared zero times:
//! `oslo.completion.spec` describes flags for Tab and parses nothing; a recipe's `params` parses and
//! offers nothing; `register_builtin` gets raw argv and does neither. The obvious fix is a fourth
//! format — which is one more than there already are.
//!
//! argc already parses, renders `--help`, and is already the Tab source through
//! `startup/repl/argc.rs`. So this is `argc::eval` with a detached runtime, and a recipe describes
//! its arguments exactly the way a script written for the `argc` builtin does.
//!
//! The last test is the one that matters most: it runs inside a registered builtin, which is where
//! a config wants this and where nothing that touches the shell may go.

#![cfg(feature = "argc")]

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
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The declaration every case below uses, written the way a script would write it.
const SPEC: &str = r#"local SPEC = [[
# @describe  Put a build somewhere
# @option -t --tries <NUM>   how many times to retry
# @flag   -n --dry-run       say what would happen
# @arg    target!            where to
]]
"#;

fn with_spec(body: &str) -> String {
    format!("{SPEC}{body}")
}

#[test]
fn an_option_and_a_positional_come_back_by_name() {
    let said = lua(&with_spec(
        r#"
local got = oslo.args.parse(SPEC, { "deploy", "--tries", "3", "prod" })
print("tries=" .. tostring(got.tries) .. " target=" .. tostring(got.target))
"#,
    ));
    assert!(said.contains("tries=3 target=prod"), "{said}");
}

/// **A dash becomes an underscore**, because `got["dry-run"]` is the only way to read it otherwise
/// and `got.dry_run` is what anybody writes first.
#[test]
fn a_dashed_flag_is_readable_as_an_identifier() {
    let said = lua(&with_spec(
        r#"
local got = oslo.args.parse(SPEC, { "deploy", "--dry-run", "prod" })
print("dry=" .. tostring(got.dry_run))
"#,
    ));
    assert!(said.contains("dry=1"), "{said}");
}

/// A missing required argument is an answer, not a raise: finding out is what the call is for.
#[test]
fn a_usage_mistake_answers_rather_than_raising() {
    let said = lua(&with_spec(
        r#"
local got, why, status = oslo.args.parse(SPEC, { "deploy" })
print("failed=" .. tostring(got == nil))
-- argc renders a positional in the upper case its usage line uses.
print("names_it=" .. tostring(tostring(why):find("TARGET") ~= nil))
print("status=" .. tostring(status))
"#,
    ));
    assert!(said.contains("failed=true"), "{said}");
    assert!(
        said.contains("names_it=true"),
        "the message does not say what is missing\n{said}"
    );
    assert!(said.contains("status=1"), "{said}");
}

/// `--help` is a status of 0 — it is what was asked for, not a mistake.
#[test]
fn help_is_success_and_carries_the_text() {
    let said = lua(&with_spec(
        r#"
local got, text, status = oslo.args.parse(SPEC, { "deploy", "--help" })
print("status=" .. tostring(status))
print("describes=" .. tostring(tostring(text):find("Put a build somewhere") ~= nil))
"#,
    ));
    assert!(said.contains("status=0"), "{said}");
    assert!(said.contains("describes=true"), "{said}");
}

#[test]
fn usage_renders_without_parsing_anything() {
    let said = lua(&with_spec(
        r#"
local text = oslo.args.usage(SPEC, "deploy")
print("names_the_command=" .. tostring(text:find("deploy") ~= nil))
print("lists_the_option=" .. tostring(text:find("tries") ~= nil))
"#,
    ));
    assert!(said.contains("names_the_command=true"), "{said}");
    assert!(said.contains("lists_the_option=true"), "{said}");
}

#[test]
fn a_list_that_does_not_start_with_a_name_is_refused() {
    let said = lua(&with_spec(
        r#"
local ok, err = pcall(function() return oslo.args.parse(SPEC, {}) end)
print("refused=" .. tostring(not ok))
print("says_why=" .. tostring(tostring(err):find("name") ~= nil))
"#,
    ));
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("says_why=true"), "{said}");
}

/// **The case the whole thing is for.** A registered builtin holds the shell, so everything that
/// borrows it raises — and parsing its own argv must not be one of those things.
#[test]
fn it_works_from_inside_a_registered_builtin() {
    let said = lua(&with_spec(
        r#"
oslo.register_builtin{ name = "deploy", run = function(argv, shell)
  local got, why = oslo.args.parse(SPEC, argv)
  if not got then print("usage: " .. tostring(why)) return 2 end
  print("target=" .. tostring(got.target) .. " tries=" .. tostring(got.tries))
end }
oslo.proc.exec("deploy --tries 5 prod")
oslo.proc.exec("deploy")
"#,
    ));
    assert!(
        said.contains("target=prod tries=5"),
        "parsing from a builtin failed\n{said}"
    );
    assert!(
        said.contains("usage: "),
        "the mistake path did not report\n{said}"
    );
}

/// A build without the feature has no namespace at all, which is the documented way to ask.
#[test]
fn the_namespace_is_the_thing_a_config_guards_on() {
    let said = lua("print('present=' .. tostring(oslo.args ~= nil))");
    assert!(said.contains("present=true"), "{said}");
}
