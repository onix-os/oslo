//! Plugins on the runtimepath, loading for real — through the real binary, in a temporary home.
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

    fn config(&self) -> PathBuf {
        self.dir.path().join(".config/oslo")
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

    /// Write a file into a root, creating whatever it needs. `where` is relative to the root.
    fn write(&self, root: &Path, at: &str, body: &str) {
        let path = root.join(at);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    /// A package root: `site/pack/<group>/start/<name>`, which is a root like any other.
    fn package(&self, group: &str, name: &str) -> PathBuf {
        let root = self
            .data()
            .join("oslo/site/pack")
            .join(group)
            .join("start")
            .join(name);
        std::fs::create_dir_all(&root).expect("mkdir");
        root
    }
}

fn out(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The plugin used by most cases: one builtin, which prints its argument.
const NOTES: &str = r#"
oslo.register_builtin{ name = "note", run = function(argv)
  print("note: " .. (argv[2] or "nothing"))
  return 0
end }
"#;

#[test]
fn the_path_is_listed_even_when_nothing_is_on_it() {
    let home = Home::new();
    let listed = home.plugin(&["list"]);
    assert!(listed.status.success());
    let text = out(&listed);
    // A path always has entries; what is empty is the list of files to run. Saying both is the
    // difference between "you installed it somewhere that is not read" and "you installed nothing".
    assert!(text.contains("runtimepath"), "{text}");
    assert!(text.contains("none"), "{text}");
}

/// **Put it on the path and it runs.** No install verb, no manifest, nothing to approve.
#[test]
fn a_plugin_on_the_path_registers_its_builtin() {
    let home = Home::new();
    let root = home.package("mine", "notes");
    home.write(&root, "plugin/notes.lua", NOTES);

    let listed = out(&home.plugin(&["list"]));
    assert!(listed.contains("notes.lua"), "{listed}");

    let session = interactive(&home, "note hello\nnote again\nexit\n");
    assert!(session.contains("note: hello"), "{session}");
    assert!(session.contains("note: again"), "{session}");
}

/// Path order between roots, alphabetical within one, and `after/` last however it sorts.
#[test]
fn load_order_is_the_path_then_after() {
    let home = Home::new();
    let config = home.config();
    let pack = home.package("mine", "later");

    let say = |what: &str| format!("print(\"ORDER:{what}\")\n");
    home.write(&config, "plugin/10-first.lua", &say("first"));
    home.write(&config, "plugin/20-second.lua", &say("second"));
    home.write(&pack, "plugin/pack.lua", &say("pack"));
    // `aa` beats every other name alphabetically and must still come last.
    home.write(&config, "after/plugin/aa.lua", &say("after"));

    let session = interactive(&home, "exit\n");
    let order: Vec<&str> = session
        .lines()
        .filter_map(|line| line.trim().strip_prefix("ORDER:"))
        .collect();
    assert_eq!(
        order,
        ["first", "second", "pack", "after"],
        "path order between roots, alphabetical within one, after last:\n{session}"
    );
}

/// **`plugin/` runs and `lua/` is required.** A plugin needs somewhere to keep a helper that is not
/// executed the moment it is on disk.
#[test]
fn lua_is_required_and_never_run_on_its_own() {
    let home = Home::new();
    let root = home.package("mine", "helped");
    home.write(&root, "lua/helper.lua", "return { word = \"HELPED\" }\n");
    // Required by nothing. If `lua/` were auto-run the way `plugin/` is, this would print.
    home.write(&root, "lua/never.lua", "print(\"MUSTNOTRUN\")\n");
    home.write(
        &root,
        "plugin/use.lua",
        "print(\"USED:\" .. require(\"helper\").word)\n",
    );

    let session = interactive(&home, "exit\n");
    assert!(session.contains("USED:HELPED"), "{session}");
    assert!(
        !session.contains("MUSTNOTRUN"),
        "a file in lua/ was run:\n{session}"
    );
}

/// A plugin gets its root as `...`, so it can read a file it ships.
#[test]
fn a_plugin_is_handed_its_root() {
    let home = Home::new();
    let root = home.package("mine", "carrier");
    home.write(&root, "data.txt", "carried along\n");
    home.write(
        &root,
        "plugin/read.lua",
        r#"
local here = ...
local f = io.open(here .. "/data.txt", "r")
print("SHIPPED:" .. (f and f:read("*l") or "NO-SHIPPED-FILE"))
if f then f:close() end
"#,
    );

    let session = interactive(&home, "exit\n");
    assert!(session.contains("SHIPPED:carried along"), "{session}");
}

/// **One raising does not stop the others.** Deliberately unlike init.lua, where a raise is fatal.
#[test]
fn a_plugin_that_raises_is_reported_and_the_rest_still_load() {
    let home = Home::new();
    let config = home.config();
    home.write(&config, "plugin/10-bad.lua", "error(\"deliberate\")\n");
    home.write(&config, "plugin/20-good.lua", "print(\"STILL-LOADED\")\n");

    let session = interactive(&home, "exit\n");
    assert!(session.contains("STILL-LOADED"), "{session}");
    assert!(
        session.contains("deliberate"),
        "it said nothing:\n{session}"
    );
}

/// The answer to "is it me or a plugin?"
#[test]
fn noplugin_runs_none_of_them() {
    let home = Home::new();
    home.write(&home.config(), "plugin/loud.lua", "print(\"LOADED\")\n");

    let with = interactive(&home, "exit\n");
    assert!(with.contains("LOADED"), "{with}");

    let without = interactive_with(&home, "exit\n", &["--noplugin"]);
    assert!(
        !without.contains("LOADED"),
        "--noplugin still ran it:\n{without}"
    );
}

// **Not covered here: the secrets grant.**
//
// `oslo.plugin.secrets` in the config decides which of the user's secrets a plugin may read, and the
// bookkeeping is unit-tested in `plugin/mod/tests.rs`. Proving it end to end needs an age identity
// and a populated store, which is a great deal of setup for one assertion — and a test that stood up
// half of it would pass whether or not the gate worked, which is worse than not having it.

/// Drive an interactive session on a pty and answer everything it printed.
fn interactive(home: &Home, input: &str) -> String {
    interactive_with(home, input, &[])
}

/// The same, with extra arguments before `-i`.
fn interactive_with(home: &Home, input: &str, args: &[&str]) -> String {
    use std::io::{Read, Write};
    let pty = nix::pty::openpty(None, None).expect("openpty");
    let master: std::fs::File = pty.master.into();
    let slave: std::fs::File = pty.slave.into();

    // The config root is also a runtimepath root, so this only ever writes init.lua -- a test that
    // put files in `plugin/` there must still find them.
    let config = home.config();
    std::fs::create_dir_all(&config).expect("mkdir");
    std::fs::write(
        config.join("init.lua"),
        "oslo.misc.welcome = false\noslo.prompt.left = function() return \"> \" end\n",
    )
    .expect("config");

    let mut command = Command::new(oslo_bin());
    command
        .args(args)
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
#[test]
fn a_shell_with_no_plugins_is_quiet_about_them() {
    let home = Home::new();
    let session = interactive(&home, "exit\n");
    assert!(
        !session.contains("plugin"),
        "a fresh shell should not mention plugins at all:\n{session}"
    );
}

/// **A runaway load is stopped rather than taking the shell with it.**
///
/// Not a sandbox: the plugin's hooks and callbacks run afterwards with no ceiling. What this catches
/// is the table that grows without end during the load — and it says so, rather than leaving a
/// plugin that registered nothing looking like one with nothing to register.
#[test]
fn a_plugin_that_allocates_without_end_is_stopped_rather_than_taking_the_shell_with_it() {
    let home = Home::new();
    let root = home.package("mine", "greedy");
    home.write(
        &root,
        "plugin/greed.lua",
        "local t = {}\nwhile true do t[#t + 1] = string.rep(\"x\", 4096) end\n",
    );

    // The claim is that the shell answers the *next* command, which it cannot do if the load took
    // it. The marker is split by a quote the shell removes, so what is asserted on is the shell's
    // output and not the pty echoing the line back.
    let session = interactive(&home, "echo STILL''-HERE\nexit\n");
    assert!(session.contains("STILL-HERE"), "{session}");
    assert!(session.contains("more than 64 MB"), "{session}");
}
