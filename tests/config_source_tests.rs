//! Reading a *shell* file from the Lua config.
//!
//! The config is Lua, and that is the one rule oslo makes. But a person's `~/.profile` and their
//! pile of `aliases.sh` are shell, and are shared with every other shell on the machine —
//! rewriting those in Lua is not an answer, so `oslo.source` reads them where they are.
//!
//! Sourced rather than run: `oslo.run{"sh", path}` executes the file in a child, where every
//! export, alias and function it defines dies with the process.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Drive the REPL with a throwaway `$HOME`, as `startup_tests` does.
fn repl(input: &str, vars: &[(&str, &str)], home: &Path) -> Output {
    let mut cmd = Command::new(oslo_bin());
    cmd.arg("-i")
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("ENV")
        .env_remove("PS1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().expect("spawn oslo -i");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write to oslo");
    child.wait_with_output().expect("oslo output")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// **`oslo.source` runs a shell file in *this* shell.**
///
/// The config is Lua, but a person's `~/.profile` and their `aliases.sh` are shell and are shared
/// with every other shell they use. Rewriting those in Lua is not an answer; reading them is.
///
/// Sourced rather than run: `oslo.run{"sh", path}` would execute the file in a child, where every
/// export, alias and function it defines dies with the process. All three have to survive.
#[test]
fn a_shell_file_can_be_sourced_from_the_lua_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".config/oslo");
    std::fs::create_dir_all(&config).unwrap();

    let profile = dir.path().join("profile.sh");
    std::fs::write(
        &profile,
        "export SOURCED_VAR=from-profile\nalias sourced_alias='echo aliased'\nsourced_fn() { echo \"fn:$1\"; }\n",
    )
    .unwrap();
    std::fs::write(
        config.join("config.lua"),
        format!("oslo.source({:?})\n", profile.to_str().unwrap()),
    )
    .unwrap();

    let o = repl(
        "echo \"var=[$SOURCED_VAR]\"\nsourced_alias\nsourced_fn hi\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    let text = out(&o);
    assert!(text.contains("var=[from-profile]"), "export lost: {text:?}");
    assert!(text.contains("aliased"), "alias lost: {text:?}");
    assert!(text.contains("fn:hi"), "function lost: {text:?}");
}

/// A file that is not there is reported and the rest of the config still loads. A missing
/// `~/.profile` is the ordinary case on a fresh machine, not a reason to start without a config.
#[test]
fn sourcing_a_missing_file_does_not_stop_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".config/oslo");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(
        config.join("config.lua"),
        "oslo.source('/no/such/profile')\noslo.env.set_alias('after', 'echo still-loaded')\n",
    )
    .unwrap();

    let o = repl("after\n", &[("HISTFILE", "")], dir.path());
    assert!(
        out(&o).contains("still-loaded"),
        "the config stopped at the missing file: {:?}",
        err(&o)
    );
    assert!(
        err(&o).contains("/no/such/profile"),
        "the missing file was not reported: {:?}",
        err(&o)
    );
}
