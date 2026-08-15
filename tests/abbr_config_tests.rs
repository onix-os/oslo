//! An abbreviation the config defined, and where it is allowed to fire.
//!
//! `oslo.abbr.NAME = { "…", anywhere = true }` is documented in the README and in
//! `docs/features/abbreviations.md`. It read correctly, installed correctly — and was then undone
//! before the first prompt, because `macros::live::want()` merges the *snapshot this shell has just
//! written of its own config* with the macro database, and `startup::stored` re-added everything it
//! got back at command position. The config was competing with its own echo.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn repl_with_config(config: &str, script: &str, home: &Path) -> Output {
    std::fs::create_dir_all(home.join(".config/oslo")).expect("config dir");
    std::fs::write(home.join(".config/oslo/config.lua"), config).expect("write config");
    // A directory of its own to run in, so a `.env.lua` anywhere above cannot join in.
    let work = home.join("work");
    std::fs::create_dir_all(&work).expect("work dir");

    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("ENV")
        .env("TERM", "dumb")
        .current_dir(&work)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("oslo starts");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(script.as_bytes())
        .expect("feed the shell");
    child.wait_with_output().expect("oslo exits")
}

/// **The flag survives startup.** `abbr` prints `--anywhere` for the one that has it and not for
/// the one that does not, which is the whole of the difference.
#[test]
fn a_config_abbreviation_keeps_the_placement_it_asked_for() {
    let home = tempfile::tempdir().expect("tempdir");
    let out = repl_with_config(
        "oslo.abbr.gco = \"git checkout\"\noslo.abbr.L = { \"| less\", anywhere = true }\n",
        "abbr\nexit\n",
        home.path(),
    );
    let printed = String::from_utf8_lossy(&out.stdout);

    assert!(
        printed.contains("abbr --anywhere L"),
        "the `anywhere` flag was lost between the config and the first prompt: {printed}"
    );
    assert!(
        printed.contains("abbr gco"),
        "the ordinary one should stay at command position: {printed}"
    );
    assert!(
        !printed.contains("abbr --anywhere gco"),
        "and must not be widened by the fix: {printed}"
    );
}
