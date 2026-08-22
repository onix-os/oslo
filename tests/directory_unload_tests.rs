//! A `.env.lua` gets a moment to clean up, and a say in what counts as a change.
//!
//! # What was missing
//!
//! Variables and aliases are put back by the undo record. **Everything else a directory environment
//! did had no moment at which to be undone** — a completion it registered, a marker it wrote, a
//! background job it started. And a file could not say that anything *other than itself* should
//! trigger a reload, so a `.tool-versions` or a lockfile changing went unnoticed.
//!
//! The machinery for the second one was already there and already drained for both file kinds;
//! nothing on the Lua side ever filled the list, so for a `.env.lua` it was permanently empty.
//! `.envrc` has had `watch_file` since the beginning.
//!
//! # Why the callback can use the whole API
//!
//! `Direnv::unload` calls the restore hook **before** it takes the environment lock, deliberately,
//! so a prompt function reading a variable sees the directory's value while it is still set. The
//! unload callbacks ride on that same moment: `oslo.env.get`, `oslo.run` and the rest all work
//! there, which is not true inside a registered builtin.

#![cfg(feature = "direnv")]

mod common;

use common::oslo_bin;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Sandbox {
    home: tempfile::TempDir,
    project: PathBuf,
    other: PathBuf,
}

impl Sandbox {
    /// A project holding `body`, and a second directory beside it to walk into.
    fn new(body: &str) -> Sandbox {
        let home = tempfile::tempdir().expect("tempdir");
        let root = home.path().canonicalize().expect("canonical");
        let project = root.join("proj");
        let other = root.join("other");
        std::fs::create_dir_all(project.join("deep")).expect("deep");
        std::fs::create_dir_all(&other).expect("other");
        std::fs::write(project.join(".env.lua"), body).expect("write");
        std::fs::write(other.join(".env.lua"), "oslo.env.set(\"OTHER\", \"yes\")\n")
            .expect("other");
        let sandbox = Sandbox {
            home,
            project,
            other,
        };
        for dir in [&sandbox.project, &sandbox.other] {
            let allowed = sandbox.run(dir, &["direnv", "allow"]);
            assert!(
                allowed.status.success(),
                "could not allow {}",
                dir.display()
            );
        }
        sandbox
    }

    fn write(&self, rel: &str, body: &str) {
        std::fs::write(self.project.join(rel), body).expect("write");
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new(oslo_bin())
            .args(args)
            .current_dir(cwd)
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env_remove("ENV")
            .stdin(Stdio::null())
            .output()
            .expect("spawn oslo")
    }

    /// Drive an interactive session from `cwd`. Down stdin, because a directory environment is
    /// loaded by the read loop before each prompt and `-c` never enters one.
    fn session(&self, cwd: &Path, lines: &str) -> String {
        let mut child = Command::new(oslo_bin())
            .arg("-i")
            .current_dir(cwd)
            .env("HOME", self.home.path())
            .env("XDG_DATA_HOME", self.home.path().join("data"))
            .env("XDG_CONFIG_HOME", self.home.path().join("config"))
            .env_remove("ENV")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(lines.as_bytes())
            .expect("write");
        let out = child.wait_with_output().expect("wait");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }
}

#[test]
fn on_unload_runs_when_the_directory_is_left() {
    let sandbox = Sandbox::new(
        r#"
oslo.direnv.on_unload(function() print("LEFT") end)
"#,
    );
    let said = sandbox.session(&sandbox.project, "cd ../other\necho done\n");
    assert!(said.contains("LEFT"), "the callback never ran\n{said}");
}

/// **It runs before the variables are put back**, so a callback can still read what the directory
/// set — which is what `Direnv::unload` calls the restore hook early for.
#[test]
fn the_callback_sees_the_directorys_variables_still_set() {
    let sandbox = Sandbox::new(
        r#"
oslo.env.set("PROJ", "here")
oslo.direnv.on_unload(function() print("SAW=" .. tostring(oslo.env.get("PROJ"))) end)
"#,
    );
    let said = sandbox.session(&sandbox.project, "cd ../other\necho done\n");
    assert!(
        said.contains("SAW=here"),
        "the callback ran after the undo, or could not reach the shell\n{said}"
    );
}

/// Registered more than once, and run in reverse: a file that registers in the order it sets things
/// up should tear down in the opposite order.
#[test]
fn callbacks_run_in_reverse_registration_order() {
    let sandbox = Sandbox::new(
        r#"
oslo.direnv.on_unload(function() print("FIRST_REGISTERED") end)
oslo.direnv.on_unload(function() print("SECOND_REGISTERED") end)
"#,
    );
    let said = sandbox.session(&sandbox.project, "cd ../other\necho done\n");
    let at = |n: &str| said.find(n).unwrap_or_else(|| panic!("{n}\n{said}"));
    assert!(
        at("SECOND_REGISTERED") < at("FIRST_REGISTERED"),
        "not LIFO\n{said}"
    );
}

/// One that raises is reported and the rest still run. By the time these fire the unload is
/// committed and the undo record is being spent; stopping half way would leave the directory's
/// variables set with nothing left to remove them.
#[test]
fn a_callback_that_raises_does_not_stop_the_others() {
    let sandbox = Sandbox::new(
        r#"
oslo.direnv.on_unload(function() print("STILL_RAN") end)
oslo.direnv.on_unload(function() error("deliberate") end)
"#,
    );
    let said = sandbox.session(&sandbox.project, "cd ../other\necho done\n");
    assert!(
        said.contains("STILL_RAN"),
        "a raise stopped the rest\n{said}"
    );
    assert!(
        said.contains("on_unload"),
        "the failure was not named\n{said}"
    );
}

/// **The incoming file's callbacks must not run as the outgoing file's.** `Direnv::arrive` unloads
/// and loads inside one call, so a single slot would fire the new registrations immediately.
#[test]
fn moving_to_another_project_runs_only_the_first_ones_callbacks() {
    let sandbox = Sandbox::new(
        r#"
oslo.direnv.on_unload(function() print("PROJ_UNLOAD") end)
"#,
    );
    std::fs::write(
        sandbox.other.join(".env.lua"),
        "oslo.direnv.on_unload(function() print(\"OTHER_UNLOAD\") end)\n",
    )
    .expect("write");
    let allowed = sandbox.run(&sandbox.other, &["direnv", "allow"]);
    assert!(allowed.status.success());
    let said = sandbox.session(&sandbox.project, "cd ../other\necho done\n");
    assert!(said.contains("PROJ_UNLOAD"), "{said}");
    assert!(
        !said.contains("OTHER_UNLOAD"),
        "the arriving file's callback fired during its own load\n{said}"
    );
}

/// A watched file changing reloads the environment, inside one session.
#[test]
fn a_watched_file_reloads_the_environment() {
    let sandbox = Sandbox::new(
        r#"
oslo.direnv.watch_file("extra.conf")
oslo.env.set("STAMP", (oslo.fs.read("extra.conf") or ""):gsub("%s+$", ""))
"#,
    );
    sandbox.write("extra.conf", "one\n");
    let said = sandbox.session(
        &sandbox.project,
        "echo first=$STAMP\nprintf two > extra.conf\ncd ..\ncd proj\necho after=$STAMP\n",
    );
    assert!(said.contains("first=one"), "{said}");
    assert!(
        said.contains("after=two"),
        "changing a watched file did not reload\n{said}"
    );
}

/// A relative watch resolves against the file's own directory, not the shell's — the same rule the
/// working directory follows since `.env.lua` started being `chdir`-ed to.
#[test]
fn a_watched_path_is_relative_to_the_file() {
    let sandbox = Sandbox::new(
        r#"
oslo.direnv.watch_file("extra.conf")
oslo.env.set("STAMP", (oslo.fs.read("extra.conf") or ""):gsub("%s+$", ""))
"#,
    );
    sandbox.write("extra.conf", "one\n");
    let deep = sandbox.project.join("deep");
    let said = sandbox.session(&deep, "echo entered=$STAMP\n");
    assert!(
        said.contains("entered=one"),
        "entered from a subdirectory and the watch did not resolve\n{said}"
    );
}

/// **Refused outside a load**, because the list is drained by the *next* arrival — so a call from a
/// timer or a spawn callback would quietly attach a path to an unrelated project.
#[test]
fn watching_outside_a_load_is_refused() {
    let home = tempfile::tempdir().expect("tempdir");
    let probe = home.path().join("probe.lua");
    std::fs::write(
        &probe,
        r#"
local ok, err = pcall(function() oslo.direnv.watch_file("x") end)
print("watch_refused=" .. tostring(not ok) .. " " .. tostring(err))
local ok2 = pcall(function() oslo.direnv.on_unload(function() end) end)
print("unload_refused=" .. tostring(not ok2))
"#,
    )
    .expect("write");
    let out = Command::new(oslo_bin())
        .arg(&probe)
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(said.contains("watch_refused=true"), "{said}");
    assert!(
        said.contains("only while"),
        "the reason is not named\n{said}"
    );
    assert!(said.contains("unload_refused=true"), "{said}");
}

/// A completion a project registered goes away with the project — which is what `forget` exists
/// for, and what `on_unload` exists to call.
#[test]
fn a_project_can_take_its_completion_back() {
    let sandbox = Sandbox::new(
        r#"
oslo.completion.spec{ command = "projtool", desc = "only here" }
oslo.direnv.on_unload(function()
  print("FORGOT=" .. tostring(oslo.completion.forget("projtool")))
end)
"#,
    );
    let said = sandbox.session(&sandbox.project, "cd ../other\necho done\n");
    assert!(
        said.contains("FORGOT=1"),
        "the spec was not there to forget\n{said}"
    );
}

/// And `forget` on a name nothing registered answers 0 rather than claiming a removal.
#[test]
fn forgetting_an_unknown_name_answers_zero() {
    let home = tempfile::tempdir().expect("tempdir");
    let probe = home.path().join("probe.lua");
    std::fs::write(
        &probe,
        r#"
oslo.completion.provider{ name = "mine", answer = function() return { "x" } end }
print("mine=" .. tostring(oslo.completion.forget("mine")))
print("nobody=" .. tostring(oslo.completion.forget("nobody")))
"#,
    )
    .expect("write");
    let out = Command::new(oslo_bin())
        .arg(&probe)
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    let said = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(said.contains("mine=1"), "{said}");
    assert!(said.contains("nobody=0"), "{said}");
}
