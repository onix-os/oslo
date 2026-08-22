//! `oslo macros --var` through the real binary: a variable whose body is a recipe.
//!
//! The fifth kind, and the only one that is *not* a value when it is stored. What is tested here is
//! when the body runs — which is the whole difference between this and an `export` in a config.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::process::{Command, Stdio};

/// A store of its own, and a shell that reads it.
struct Shell {
    data: tempfile::TempDir,
}

impl Shell {
    fn new() -> Shell {
        Shell {
            data: tempfile::tempdir().expect("tempdir"),
        }
    }

    /// `oslo macros …` against this store.
    fn macros(&self, args: &[&str]) -> String {
        let out = Command::new(oslo_bin())
            .arg("macros")
            .args(args)
            .env("XDG_DATA_HOME", self.data.path())
            .env("XDG_CONFIG_HOME", self.data.path().join("config"))
            .env("OSLO_MACROS_BIN", self.data.path().join("bin"))
            .stdin(Stdio::null())
            .output()
            .expect("spawn oslo");
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Type `line` at an interactive shell that has this store.
    fn at_a_prompt(&self, line: &str) -> String {
        let mut child = Command::new(oslo_bin())
            .arg("-i")
            .env("XDG_DATA_HOME", self.data.path())
            .env("XDG_CONFIG_HOME", self.data.path().join("config"))
            .env("OSLO_MACROS_BIN", self.data.path().join("bin"))
            .env_remove("GITHUB_TOKEN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn oslo");
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(format!("{line}\n").as_bytes())
            .expect("write");
        let out = child.wait_with_output().expect("wait");
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

/// **A variable may be written the way it would be typed.** `NAME=body` as one word is what
/// somebody's fingers already know, and it stores the same thing as a name and a body.
#[test]
fn a_variable_can_be_written_as_one_word() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "PLACE=/srv/data"]);

    let listed = shell.macros(&["export"]);
    assert!(listed.contains("var PLACE"), "{listed:?}");
    assert!(shell.at_a_prompt("echo [$PLACE]").contains("[/srv/data]"));
}

/// The same thing, as a name and a body.
#[test]
fn a_variable_can_be_written_as_two_words() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "PLACE", "/srv/data"]);
    assert!(shell.at_a_prompt("echo [$PLACE]").contains("[/srv/data]"));
}

/// **The body is a recipe, and it runs in the shell that reads the name.**
#[test]
fn the_body_runs_when_the_name_is_read() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "GREETING=$(echo made-on-demand)"]);
    let out = shell.at_a_prompt("echo [$GREETING]");
    assert!(out.contains("[made-on-demand]"), "{out:?}");
}

/// **Nothing runs until something asks.** A shell that never mentions the name never runs the body
/// — which is what makes `$(oslo secret get …)` free to keep, and what an `export` in a config
/// could never be: that decrypts every secret at every start, for every shell, forever.
#[test]
fn a_variable_nobody_reads_costs_nothing() {
    let shell = Shell::new();
    let marker = shell.data.path().join("it-ran");
    shell.macros(&[
        "add",
        "--var",
        &format!("WATCHED=$(touch {} && echo yes)", marker.display()),
    ]);

    shell.at_a_prompt("echo something-else");
    assert!(
        !marker.exists(),
        "the body ran in a shell that never mentioned the name"
    );

    let out = shell.at_a_prompt("echo [$WATCHED]");
    assert!(out.contains("[yes]"), "{out:?}");
    assert!(marker.exists(), "the body did not run when it was read");
}

/// And it runs **once**: the second read finds an ordinary variable, not a second command.
#[test]
fn the_body_runs_once_and_then_it_is_a_value() {
    let shell = Shell::new();
    let counter = shell.data.path().join("times");
    shell.macros(&[
        "add",
        "--var",
        &format!("COUNTED=$(printf x >> {} && echo done)", counter.display()),
    ]);

    shell.at_a_prompt("echo [$COUNTED] [$COUNTED] [$COUNTED]");
    let times = std::fs::read_to_string(&counter).unwrap_or_default();
    assert_eq!(times, "x", "the recipe ran {} times", times.len());
}

/// It is exported, because a variable nobody can pass to a command is not what anybody meant.
#[test]
fn the_value_reaches_a_child_process() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "PASSED=through"]);
    let out = shell.at_a_prompt("/usr/bin/env | grep '^PASSED='");
    assert!(out.contains("PASSED=through"), "{out:?}");
}

/// **A stored variable wins over the environment the shell was started with.**
///
/// This assertion used to run the other way, and the rule it recorded is the defect: `EDITOR`
/// arrives from a terminal emulator or a session manager, so `oslo macros add --var EDITOR=nvim`
/// reported success and then changed nothing, for ever. Alias and Abbrev are applied in the same
/// loop and never deferred; the `macros.sh` rendering of the very same entry is an unguarded
/// `export`. `Var` was the odd one out in both directions.
///
/// `FOO=x oslo …` still means what it always meant for every name nobody has deliberately stored,
/// which is all of them but a handful.
#[test]
fn a_stored_variable_beats_the_environment_it_was_started_with() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "ALREADY=from-the-store"]);

    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("XDG_DATA_HOME", shell.data.path())
        .env("XDG_CONFIG_HOME", shell.data.path().join("config"))
        .env("ALREADY", "from-the-parent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo [$ALREADY]\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("[from-the-store]"),
        "the stored variable lost to the parent's environment\n{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// **The symmetry, as one assertion.** A stored var and a stored alias meet the same collision —
/// the config sets the same name — and must come out the same way. They did not: the alias won and
/// the var lost, and nothing distinguished the two cases but which arm of one `match` they hit.
#[test]
fn a_var_and_an_alias_beat_the_config_alike() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "GREET=from-macro"]);
    shell.macros(&["add", "--alias", "ll", "echo from-macro"]);
    std::fs::create_dir_all(shell.data.path().join("config/oslo")).expect("config dir");
    std::fs::write(
        shell.data.path().join("config/oslo/init.lua"),
        "oslo.proc.exec('alias ll=\"echo from-init-lua\"')\n\
         oslo.env.set(\"GREET\", \"from-init-lua\")\n",
    )
    .expect("init.lua");

    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("HOME", shell.data.path())
        .env("XDG_DATA_HOME", shell.data.path())
        .env("XDG_CONFIG_HOME", shell.data.path().join("config"))
        .env("GREET", "from-environment")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo VAR:$GREET\nll\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains("VAR:from-macro"), "the var lost\n{said:?}");
    assert!(
        said.matches("from-macro").count() >= 2,
        "the two kinds did not agree\n{said:?}"
    );
}

/// Removing it takes the recipe away from every shell that has not read it yet.
#[test]
fn removing_it_takes_the_recipe_back() {
    let shell = Shell::new();
    shell.macros(&["add", "--var", "TEMPORARY=here"]);
    assert!(shell.at_a_prompt("echo [$TEMPORARY]").contains("[here]"));

    shell.macros(&["remove", "--var", "TEMPORARY"]);
    assert!(shell.at_a_prompt("echo [$TEMPORARY]").contains("[]"));
}
