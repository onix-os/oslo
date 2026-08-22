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

/// **An inherited variable containing a space survives an interactive shell.**
///
/// This is the regression that broke a prompt. Every exported variable is written into
/// `elsewhere.snapshot` so the manager can list what is defined that you did not store — and that
/// list is applied back. While a stored variable deferred to one already set, applying it was a
/// no-op. Once stored variables started winning, the round trip became live, and `is_a_value`
/// classified anything containing whitespace as a *recipe*: `SSH_CONNECTION` was unset and replaced
/// by the output of trying to execute an IP address, so `$SSH_CONNECTION` came back as `1.2.3.4` and
/// a prompt segment keyed on it went blank.
///
/// Interactive, because the macro store is only applied by the REPL — `oslo -c` never showed it.
#[test]
fn an_inherited_value_with_spaces_is_not_run_as_a_command() {
    let shell = Shell::new();
    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("XDG_DATA_HOME", shell.data.path())
        .env("XDG_CONFIG_HOME", shell.data.path().join("config"))
        .env("SSH_CONNECTION", "1.2.3.4 22 5.6.7.8 22")
        .env("WITHSPACE", "alpha beta gamma")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo [$SSH_CONNECTION] [$WITHSPACE]\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("[1.2.3.4 22 5.6.7.8 22]"),
        "the value was cut at the first space, or run as a command\n{said:?}"
    );
    assert!(said.contains("[alpha beta gamma]"), "{said:?}");
}

/// **A variable the configuration set is a value, whatever it looks like.**
///
/// `oslo.env.set` stores a literal string. What broke it was a round trip: every exported variable a
/// starting shell holds is written to `elsewhere.snapshot` so `oslo macros` can list what is defined
/// that you did not store — and that file was then read back and *applied*. A body containing `$(…)`
/// reads as a recipe on the way in, so the variable was unset, re-registered as a recipe, and came
/// back as its first word.
///
/// `eval "$(direnv export bash)"` — the direnv hook, and the shape that found this — became `eval`.
#[test]
fn a_configured_value_is_not_re_read_as_a_recipe() {
    let shell = Shell::new();
    let config = shell.data.path().join("config/oslo");
    std::fs::create_dir_all(&config).expect("config dir");
    std::fs::write(
        config.join("init.lua"),
        "oslo.misc.welcome = false\n\
         oslo.env.set(\"HOOKLINE\", 'eval \"$(echo inner)\"')\n\
         oslo.env.set(\"SPACED\", \"a $(echo hi) b\")\n",
    )
    .expect("init.lua");

    let out = shell.at_a_prompt("echo [$HOOKLINE] [$SPACED]");
    assert!(
        out.contains(r#"[eval "$(echo inner)"]"#),
        "the hook line was cut or run\n{out:?}"
    );
    assert!(
        out.contains("[a $(echo hi) b]"),
        "the value was cut or run\n{out:?}"
    );
}

/// **One shell's environment does not reach another one.**
///
/// This passed before `want()` stopped applying them too, and it is worth pinning anyway, because
/// what held it up was nothing to do with intent: `elsewhere.snapshot` holds every exported
/// variable a shell had — `PATH`, `IN_NIX_SHELL`, the `CC`/`AR`/`CONFIG_SHELL` of whatever dev
/// shell a terminal was in, and `OSLO_DIRENV`, a directory environment's *undo record*. Two
/// accidents kept them from travelling: a starting shell rewrites the file before it reads it, and
/// `Stamps::now` watches the macro database and the session file but not this one, so a running
/// shell never re-reads it. Change either and the whole environment of one terminal would arrive in
/// the next. Now nothing in it is applied, and this says so.
#[test]
fn a_variable_one_shell_held_is_not_applied_to_the_next() {
    let shell = Shell::new();
    // The first shell publishes its environment, which is what writes the snapshot.
    let first = shell.at_a_prompt("echo [$OSLO_T_LEAK]");
    assert!(
        first.contains("[]"),
        "the marker must start unset: {first:?}"
    );

    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("XDG_DATA_HOME", shell.data.path())
        .env("XDG_CONFIG_HOME", shell.data.path().join("config"))
        .env("OSLO_T_LEAK", "from-the-first-shell")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo [$OSLO_T_LEAK]\n")
        .expect("write");
    let held = child.wait_with_output().expect("wait");
    assert!(
        String::from_utf8_lossy(&held.stdout).contains("[from-the-first-shell]"),
        "the shell that has it must still see it"
    );

    // A third shell, without it in its environment, must not inherit it through the store.
    let after = shell.at_a_prompt("echo [$OSLO_T_LEAK]");
    assert!(
        after.contains("[]"),
        "one shell's variable travelled to another through elsewhere.snapshot\n{after:?}"
    );
}
