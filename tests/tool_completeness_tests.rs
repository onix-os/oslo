//! Every tool the main menu advertises does something.
//!
//! # What this exists to catch
//!
//! `oslo --help` listed `hook` as "list and test the shell hooks" and `direnv` as "manage
//! per-directory environments". Both answered:
//!
//! ```text
//! SUBCOMMANDS
//!   none yet — this tool is not implemented
//! ```
//!
//! The tool table's own comment says the dispatcher and the help read one list "so a tool cannot be
//! reachable and undocumented, **or listed and unreachable**". That invariant was true of the
//! plumbing and false of the tools, because nothing checked the other half of it — whether the
//! thing on the end of the name exists.
//!
//! So this walks the advertised names and asks each one for its subcommands. A tool that has none
//! is a promise in the help that nothing keeps.

mod common;

use common::oslo_bin;
use std::process::{Command, Output, Stdio};

fn oslo(args: &[&str]) -> Output {
    Command::new(oslo_bin())
        .args(args)
        .stdin(Stdio::null())
        .env("HOME", "/nonexistent-oslo-test-home")
        .output()
        .expect("spawn oslo")
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The tool names the main menu offers, read from the menu itself rather than from a second list.
///
/// The heading carries prose of its own — `TOOLS  a script of the same name always wins` — so the
/// rest of that line is dropped before the rows are read, and the rows end at the first blank.
fn advertised() -> Vec<String> {
    let help = text(&oslo(&["--help"]));
    let after = help
        .split_once("TOOLS")
        .expect("the main menu has a TOOLS section")
        .1;
    let rows = after.split_once('\n').expect("rows follow the heading").1;
    let names: Vec<String> = rows
        .lines()
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.strip_prefix("  "))
        .filter_map(|line| line.split_whitespace().next())
        .map(str::to_string)
        .collect();
    // A parser that quietly read nothing would make every test below pass for the wrong reason.
    assert!(
        names.iter().any(|n| n == "macros") && names.len() >= 5,
        "the TOOLS section was not parsed: {names:?}"
    );
    names
}

/// **The invariant the tool table claims.** A name in the menu that answers "not implemented" is
/// worse than no name at all: it is documentation for something that is not there.
#[test]
fn no_advertised_tool_is_hollow() {
    let hollow: Vec<String> = advertised()
        .into_iter()
        .filter(|tool| text(&oslo(&[tool, "--help"])).contains("not implemented"))
        .collect();
    assert!(
        hollow.is_empty(),
        "the main menu advertises these and they do nothing: {hollow:?}"
    );
}

/// Every advertised tool answers `--help` with a page, not an error.
#[test]
fn every_advertised_tool_has_a_help_page() {
    for tool in advertised() {
        let out = oslo(&[&tool, "--help"]);
        let page = text(&out);
        assert!(
            page.contains("USAGE") || page.contains("usage"),
            "`oslo {tool} --help` printed no usage: {page:?}"
        );
    }
}

// ------------------------------------------------------------------ oslo hook

/// `list` names every hook the shell has, which is what makes a misspelt one visible.
#[test]
fn hook_list_names_the_hooks() {
    let listed = text(&oslo(&["hook", "list"]));
    for hook in ["pre-cmd", "post-change-dir", "on-job-finish", "on-key"] {
        assert!(listed.contains(hook), "`{hook}` is missing from: {listed}");
    }
}

/// `show` resolves any spelling to the one moment behind it.
#[test]
fn hook_show_resolves_an_older_spelling() {
    let shown = text(&oslo(&["hook", "show", "preexec"]));
    assert!(shown.contains("pre-cmd"), "{shown}");
    assert!(
        shown.contains("oslo.on[\"precmd\"](f)"),
        "the other spellings are not listed: {shown}"
    );
}

/// A name that is not a hook is refused, rather than reported as attached to nothing.
#[test]
fn hook_show_refuses_a_name_that_is_not_a_hook() {
    let out = oslo(&["hook", "show", "precmb"]);
    assert_eq!(out.status.code(), Some(2), "{}", text(&out));
    assert!(text(&out).contains("no such hook"), "{}", text(&out));
}

/// **`test` really fires it**, against the configuration a new session would load.
#[test]
fn hook_test_runs_the_handler_in_your_config() {
    let home = tempfile::tempdir().expect("tempdir");
    let dir = home.path().join("config/oslo");
    std::fs::create_dir_all(&dir).expect("config dir");
    std::fs::write(
        dir.join("init.lua"),
        "oslo.on[\"post-change-dir\"](function(e) print(\"went to \" .. tostring(e.to)) end)\n",
    )
    .expect("config");

    let out = Command::new(oslo_bin())
        .args(["hook", "test", "post-change-dir", "to=/tmp"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(
        text(&out).contains("went to /tmp"),
        "the handler did not run: {}",
        text(&out)
    );
}

/// An answering hook's answer is reported, and reported as a value rather than as Rust debug.
#[test]
fn hook_test_reports_what_an_answering_hook_decided() {
    let home = tempfile::tempdir().expect("tempdir");
    let dir = home.path().join("config/oslo");
    std::fs::create_dir_all(&dir).expect("config dir");
    std::fs::write(
        dir.join("init.lua"),
        "oslo.on[\"pre-cmd\"](function(e) return \"instead: \" .. tostring(e.text) end)\n",
    )
    .expect("config");

    let out = Command::new(oslo_bin())
        .args(["hook", "test", "pre-cmd", "text=ls"])
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let page = text(&out);
    assert!(page.contains("instead: ls"), "{page}");
    assert!(
        !page.contains("Str("),
        "the answer is shown as Rust debugs it: {page}"
    );
}

// ------------------------------------------------------------------ oslo direnv

/// The tool and the builtin share one store, which is the only reason the tool is worth having.
#[cfg(feature = "direnv")]
#[test]
fn direnv_allow_is_recorded_where_the_shell_reads_it() {
    let home = tempfile::tempdir().expect("tempdir");
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::write(project.join(".env.lua"), "oslo.env.set(\"X\", \"1\")\n").expect("rc");

    let run = |args: &[&str]| {
        Command::new(oslo_bin())
            .args(args)
            .current_dir(&project)
            .env("HOME", home.path())
            .env("XDG_DATA_HOME", home.path())
            .stdin(Stdio::null())
            .output()
            .expect("spawn oslo")
    };

    assert!(
        text(&run(&["direnv", "status"])).contains("not allowed"),
        "a fresh directory file starts inert"
    );
    assert!(run(&["direnv", "allow"]).status.success());
    let after = text(&run(&["direnv", "status"]));
    assert!(after.contains("allowed"), "{after}");
    assert!(after.contains("1 allowed"), "the count is wrong: {after}");

    assert!(run(&["direnv", "deny"]).status.success());
    assert!(text(&run(&["direnv", "status"])).contains("denied"));
}

/// The two subcommands that need a session say so, rather than failing as a typo would.
#[cfg(feature = "direnv")]
#[test]
fn direnv_says_which_subcommands_need_a_running_shell() {
    for session in ["reload", "edit"] {
        let out = oslo(&["direnv", session]);
        let said = text(&out);
        assert!(
            said.contains("needs a running shell"),
            "`oslo direnv {session}` said: {said}"
        );
        assert!(
            !said.contains("no such subcommand"),
            "reported as a typo instead: {said}"
        );
    }
}
