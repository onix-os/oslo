//! `oslo plugin`, and a plugin actually loading — through the real binary, in a temporary home.
//!
//! **`plugin` only**: without the feature there is no subcommand to drive.
#![cfg(feature = "plugin")]

mod common;

use common::oslo_bin;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A home with nothing in it, and the environment that points oslo at it.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Home {
        Home {
            dir: tempfile::tempdir().expect("temp dir"),
        }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    /// Run `oslo plugin …` against this home.
    fn plugin(&self, args: &[&str]) -> Output {
        let mut command = Command::new(oslo_bin());
        command
            .arg("plugin")
            .args(args)
            .env("HOME", self.path())
            .env("XDG_DATA_HOME", self.data())
            .env_remove("ENV")
            .stdin(Stdio::null());
        command.output().expect("spawn oslo")
    }

    /// Write a plugin somewhere outside the home, ready to be installed from.
    fn candidate(&self, name: &str, manifest: &str, entry: &str) -> PathBuf {
        let source = self.dir.path().join("src").join(name);
        std::fs::create_dir_all(&source).expect("mkdir");
        std::fs::write(source.join("plugin.lua"), manifest).expect("manifest");
        std::fs::write(source.join("init.lua"), entry).expect("entry");
        source
    }

    fn installed_dir(&self, name: &str) -> PathBuf {
        self.data().join("oslo/plugins").join(name)
    }
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn err(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The plugin used by most cases: one builtin, which prints its argument.
fn notes(home: &Home) -> PathBuf {
    home.candidate(
        "notes",
        r#"return { name = "notes", version = "1.0", builtins = { "note" } }"#,
        r#"
oslo.register_builtin{ name = "note", run = function(argv)
  print("note: " .. (argv[2] or "nothing"))
  return 0
end }
"#,
    )
}

#[test]
fn nothing_is_installed_to_begin_with() {
    let home = Home::new();
    let listed = home.plugin(&["list"]);
    assert!(listed.status.success());
    assert!(
        out(&listed).contains("no plugins installed"),
        "{}",
        out(&listed)
    );
}

#[test]
fn installing_says_what_it_reserves_and_records_it() {
    let home = Home::new();
    let source = notes(&home);
    let installed = home.plugin(&["install", source.to_str().unwrap(), "--yes"]);
    assert!(installed.status.success(), "{}", err(&installed));
    // **What it will claim is shown before it is trusted**, and nothing of the plugin has run.
    assert!(
        out(&installed).contains("reserves: note"),
        "{}",
        out(&installed)
    );

    assert!(home.installed_dir("notes").join("init.lua").is_file());
    let listed = home.plugin(&["list"]);
    let listed = out(&listed);
    assert!(listed.contains("notes"), "{listed}");
    assert!(listed.contains("ok"), "{listed}");
}

/// **The point of the whole design**: the plugin's Lua runs when its command is called, not before.
#[test]
fn a_declared_builtin_runs_the_plugin_on_first_use() {
    let home = Home::new();
    let source = home.candidate(
        "notes",
        r#"return { name = "notes", builtins = { "note" } }"#,
        r#"
print("loading notes")
oslo.register_builtin{ name = "note", run = function(argv)
  print("note: " .. (argv[2] or "nothing"))
  return 0
end }
"#,
    );
    assert!(
        home.plugin(&["install", source.to_str().unwrap(), "--yes"])
            .status
            .success()
    );

    // An interactive session is the only thing that loads plugins, so drive one.
    let session = interactive(&home, "note hello\nnote again\nexit\n");
    assert!(session.contains("note: hello"), "{session}");
    assert!(session.contains("note: again"), "{session}");
    assert_eq!(
        session.matches("loading notes").count(),
        1,
        "the plugin should load once per session, not once per call:\n{session}"
    );
}

/// A plugin edited after it was allowed does not run until it is allowed again.
#[test]
fn a_changed_plugin_refuses_to_load_until_it_is_allowed() {
    let home = Home::new();
    let source = notes(&home);
    assert!(
        home.plugin(&["install", source.to_str().unwrap(), "--yes"])
            .status
            .success()
    );

    // A marker the refusal message could not contain, so "did it run" is unambiguous — the refusal
    // itself ends with the words "what changed", which a laxer assertion mistook for the plugin.
    std::fs::write(
        home.installed_dir("notes").join("init.lua"),
        r#"oslo.register_builtin{ name = "note", run = function() print("EDITED-VERSION-RAN") return 0 end }"#,
    )
    .expect("edit");

    let listed = out(&home.plugin(&["list"]));
    assert!(listed.contains("CHANGED"), "{listed}");

    let session = interactive(&home, "note hello\nexit\n");
    assert!(
        session.contains("has changed since you allowed it"),
        "{session}"
    );
    assert!(
        !session.contains("EDITED-VERSION-RAN"),
        "the edited plugin ran anyway:\n{session}"
    );

    // Allowing it records the new hash, and then it runs.
    let allowed = home.plugin(&["allow", "notes"]);
    assert!(allowed.status.success(), "{}", err(&allowed));
    let session = interactive(&home, "note hello\nexit\n");
    assert!(session.contains("EDITED-VERSION-RAN"), "{session}");
}

#[test]
fn removing_takes_the_plugin_but_leaves_its_database() {
    let home = Home::new();
    let source = notes(&home);
    assert!(
        home.plugin(&["install", source.to_str().unwrap(), "--yes"])
            .status
            .success()
    );
    // A database the plugin would have written.
    let database = home.data().join("oslo/plugins/notes.kv");
    std::fs::create_dir_all(database.parent().unwrap()).expect("mkdir");
    std::fs::write(&database, b"pretend").expect("write");

    let removed = home.plugin(&["remove", "notes"]);
    assert!(removed.status.success(), "{}", err(&removed));
    assert!(
        !home.installed_dir("notes").exists(),
        "the directory stayed"
    );
    assert!(
        database.is_file(),
        "the database was deleted with the plugin"
    );
    assert!(
        out(&removed).contains("database is left"),
        "removal must say the data is still there: {}",
        out(&removed)
    );

    assert!(out(&home.plugin(&["list"])).contains("no plugins installed"));
    assert!(!home.plugin(&["remove", "notes"]).status.success());
}

/// Two plugins cannot both claim one command name.
#[test]
fn a_name_another_plugin_reserves_is_refused_at_install() {
    let home = Home::new();
    let first = notes(&home);
    assert!(
        home.plugin(&["install", first.to_str().unwrap(), "--yes"])
            .status
            .success()
    );
    let second = home.candidate(
        "diary",
        r#"return { name = "diary", builtins = { "note" } }"#,
        "-- nothing\n",
    );
    let refused = home.plugin(&["install", second.to_str().unwrap(), "--yes"]);
    assert!(!refused.status.success());
    assert!(
        err(&refused).contains("another plugin already has"),
        "{}",
        err(&refused)
    );
}

/// A plugin that declares a name and then does not register it leaves the name unclaimed.
///
/// **Not a special case, and deliberately.** The loader runs the plugin and the shell then looks the
/// word up like any other; nothing checks that the plugin delivered what its manifest promised. The
/// result is the ordinary "command not found", which is exactly what happened — the command is not
/// there.
#[test]
fn a_plugin_that_registers_nothing_leaves_the_name_unclaimed() {
    let home = Home::new();
    let source = home.candidate(
        "notes",
        r#"return { name = "notes", builtins = { "note" } }"#,
        "print('loaded but registered nothing')\n",
    );
    assert!(
        home.plugin(&["install", source.to_str().unwrap(), "--yes"])
            .status
            .success()
    );
    let session = interactive(&home, "note hello\nexit\n");
    assert!(
        session.contains("loaded but registered nothing"),
        "{session}"
    );
    assert!(session.contains("not found"), "{session}");
}

/// A plugin needing a newer oslo is refused at install, rather than installed and left to fail.
#[test]
fn a_plugin_that_needs_a_newer_oslo_is_refused() {
    let home = Home::new();
    let source = home.candidate(
        "future",
        r#"return { name = "future", builtins = { "fut" }, requires = ">= 99.0.0" }"#,
        "-- nothing\n",
    );
    let refused = home.plugin(&["install", source.to_str().unwrap(), "--yes"]);
    assert!(!refused.status.success());
    assert!(
        err(&refused).contains("needs oslo >= 99.0.0"),
        "{}",
        err(&refused)
    );
    assert!(out(&home.plugin(&["list"])).contains("no plugins installed"));
}

/// One this oslo satisfies installs and runs like any other.
#[test]
fn a_requirement_this_oslo_meets_is_no_obstacle() {
    let home = Home::new();
    let source = home.candidate(
        "notes",
        r#"return { name = "notes", builtins = { "note" }, requires = ">= 0.0.1" }"#,
        r#"oslo.register_builtin{ name = "note", run = function() print("REQUIREMENT-MET") return 0 end }"#,
    );
    assert!(
        home.plugin(&["install", source.to_str().unwrap(), "--yes"])
            .status
            .success()
    );
    let session = interactive(&home, "note\nexit\n");
    assert!(session.contains("REQUIREMENT-MET"), "{session}");
}

/// **A load that allocates without end is stopped, and the shell survives it.**
///
/// Not a sandbox — the plugin's hooks run later with no ceiling, and any of them can start a
/// command. What this is for is the plugin whose entry file would otherwise take the session down
/// with it, which is a mistake far more likely than malice. See `lua/engine/plugin.rs`.
#[test]
fn a_plugin_that_allocates_without_end_is_stopped_rather_than_taking_the_shell_with_it() {
    let home = Home::new();
    let source = home.candidate(
        "greedy",
        r#"return { name = "greedy", builtins = { "greed" } }"#,
        r#"
local hoard = {}
while true do hoard[#hoard + 1] = string.rep("x", 4096) end
oslo.register_builtin{ name = "greed", run = function() return 0 end }
"#,
    );
    assert!(
        home.plugin(&["install", source.to_str().unwrap(), "--yes"])
            .status
            .success()
    );
    // `greed` is what makes it load — plugins are read on first use of a name they declared. The
    // claim is that the shell answers the *next* command, which it cannot do if the load took it.
    //
    // The marker is split by a quote the shell removes, so what is asserted on is the shell's
    // *output* and not the pty echoing the line back.
    let session = interactive(&home, "greed\necho STILL''-HERE\nexit\n");
    assert!(session.contains("STILL-HERE"), "{session}");
    // And it says so, rather than leaving a plugin that registered nothing looking like one with
    // nothing to register.
    assert!(session.contains("more than 64 MB"), "{session}");
}

#[test]
fn a_git_source_without_a_revision_is_refused_before_anything_is_fetched() {
    let home = Home::new();
    let refused = home.plugin(&["install", "github:user/repo"]);
    assert_eq!(refused.status.code(), Some(2));
    assert!(
        err(&refused).contains("name a revision"),
        "{}",
        err(&refused)
    );
}

/// Drive an interactive session on a pty and answer everything it printed.
fn interactive(home: &Home, input: &str) -> String {
    use std::io::{Read, Write};
    let pty = nix::pty::openpty(None, None).expect("openpty");
    let master: std::fs::File = pty.master.into();
    let slave: std::fs::File = pty.slave.into();

    let config = home.path().join(".config/oslo");
    std::fs::create_dir_all(&config).expect("mkdir");
    std::fs::write(
        config.join("init.lua"),
        "oslo.misc.welcome = false\noslo.prompt.left = function() return \"> \" end\n",
    )
    .expect("config");

    let mut command = Command::new(oslo_bin());
    command
        .arg("-i")
        .env_clear()
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.data())
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "dumb")
        .current_dir(home.path())
        .stdin(Stdio::from(slave.try_clone().expect("clone")))
        .stdout(Stdio::from(slave.try_clone().expect("clone")))
        .stderr(Stdio::from(slave));
    // SAFETY: runs after fork and calls only async-signal-safe interfaces.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            if nix::libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // **The controlling terminal, without which the shell never reads its input.** Leaving
            // this out is what made the first version of this helper hang until the test timed out.
            if nix::libc::ioctl(0, nix::libc::TIOCSCTTY, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().expect("spawn oslo");

    let mut input_side = master.try_clone().expect("clone");
    input_side.write_all(input.as_bytes()).expect("write");
    input_side.flush().expect("flush");

    // Read on a thread against a deadline, so a shell that never exits fails the test instead of
    // hanging the suite.
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = master;
        let mut chunk = [0u8; 4096];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if send.send(chunk[..read].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut transcript = Vec::new();
    while let Ok(chunk) =
        receive.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
    {
        transcript.extend_from_slice(&chunk);
    }
    let _ = child.kill();
    let _ = child.wait();
    String::from_utf8_lossy(&transcript).into_owned()
}

/// **Installing a plugin over itself must not destroy it.**
///
/// For a path source `fetch` hands back the source directory itself, and the install used to delete
/// the destination before copying — so reinstalling an already-installed plugin from inside the
/// plugins directory deleted it, copied the now-empty directory over itself, and then reported that
/// the plugin had no Lua in it. The files were gone, with no backup.
#[test]
fn reinstalling_a_plugin_over_itself_keeps_its_files() {
    let home = Home::new();
    let source = notes(&home);
    let first = home.plugin(&["install", source.to_str().unwrap(), "--yes"]);
    assert!(first.status.success(), "{}", err(&first));

    // Something of the user's that only lives in the installed copy.
    let installed = home.installed_dir("notes");
    std::fs::write(installed.join("keep.txt"), "irreplaceable").expect("write");

    let again = home.plugin(&["install", installed.to_str().unwrap(), "--yes"]);
    assert!(again.status.success(), "{}", err(&again));
    assert!(
        installed.join("init.lua").is_file(),
        "the plugin's entry survived"
    );
    assert_eq!(
        std::fs::read_to_string(installed.join("keep.txt")).unwrap_or_default(),
        "irreplaceable",
        "and so did everything beside it"
    );
    // The copy is made beside the destination and moved over it; nothing is left half-installed.
    assert!(
        !home.data().join("oslo/plugins/.installing-notes").exists(),
        "no staging directory left behind"
    );
}
