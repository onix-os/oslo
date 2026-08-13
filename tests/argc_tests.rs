//! `argc` through the real binary: the builtin, `--argc-eval`, and a script's own helpers.
//!
//! These run the built oslo rather than calling into the library, and that is the point for three
//! of them: a `# @option --dir=`_fn`` is answered by *running the script's function*, which is a
//! command substitution, which forks. A unit test that forks from a process with a dozen other test
//! threads in it hangs instead of failing — observed once, which is why these live here.
#![cfg(feature = "argc")]

mod common;

use common::oslo_bin;
use std::io::Write;
use std::process::Command;

/// Write a script, make it executable, and answer with its path.
fn script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create");
    file.write_all(body.as_bytes()).expect("write");
    drop(file);
    let mut mode = std::fs::metadata(&path).expect("stat").permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&path, mode).expect("chmod");
    path
}

fn oslo(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(oslo_bin())
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

const DEMO: &str = "\
#!/usr/bin/env oslo
# @describe Deploy a thing
# @flag   -n --dry-run     say what would happen
# @option -t --tries <N>   how many times
# @arg    target!          where to
argc \"$@\"
echo \"dry=$argc_dry_run tries=$argc_tries target=$argc_target\"
";

#[test]
fn a_script_parses_its_own_arguments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = script(dir.path(), "deploy", DEMO);
    let (out, _, status) = oslo(&[path.to_str().unwrap(), "--dry-run", "-t", "3", "prod"]);
    assert_eq!(status, 0);
    assert_eq!(out.trim(), "dry=1 tries=3 target=prod");
}

/// **`--help` ends the script**, the way the bash rendering's `exit 0` does. Without that the body
/// runs with nothing set and prints `dry= tries=` under the help it just printed.
#[test]
fn help_is_generated_and_stops_there() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = script(dir.path(), "deploy", DEMO);
    let (out, _, status) = oslo(&[path.to_str().unwrap(), "--help"]);
    assert_eq!(status, 0);
    assert!(out.contains("Deploy a thing"), "{out}");
    assert!(out.contains("-t, --tries <N>"), "{out}");
    assert!(!out.contains("dry="), "the body ran anyway:\n{out}");
}

#[test]
fn a_bad_argument_is_reported_and_the_body_does_not_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = script(dir.path(), "deploy", DEMO);
    let (out, err, status) = oslo(&[path.to_str().unwrap(), "--nonsense"]);
    assert_eq!(status, 1);
    assert!(err.contains("--nonsense"), "{err}");
    assert!(!out.contains("dry="), "the body ran anyway:\n{out}");
}

/// The subcommand the arguments named is called, in this shell.
#[test]
fn a_subcommand_runs_the_function_it_names() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = script(
        dir.path(),
        "tool",
        "#!/usr/bin/env oslo\n\
         # @cmd Say hello\n\
         # @arg who!\n\
         hello() {\n\
           echo \"hello $argc_who\"\n\
         }\n\
         argc \"$@\"\n",
    );
    let (out, err, status) = oslo(&[path.to_str().unwrap(), "hello", "world"]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(out.trim(), "hello world");
}

/// **A default computed by a function runs in this shell**, not in a bash oslo had to find. That is
/// the whole reason `argc::Runtime` is implemented over the shell rather than taken from upstream.
#[test]
fn a_default_is_computed_by_the_scripts_own_function() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = script(
        dir.path(),
        "where",
        "#!/usr/bin/env oslo\n\
         # @option --dir=`_here`\n\
         _here() {\n\
           printf /somewhere\n\
         }\n\
         argc \"$@\"\n\
         echo \"dir=$argc_dir\"\n",
    );
    let (out, err, status) = oslo(&[path.to_str().unwrap()]);
    assert_eq!(status, 0, "{err}");
    assert_eq!(out.trim(), "dir=/somewhere");
}

/// `oslo --argc-eval` prints shell code a *bash* script evaluates. The assignments are what matter;
/// the shape of the rest is argc's and not oslo's to assert.
#[test]
fn argc_eval_prints_assignments_for_another_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = script(dir.path(), "deploy", DEMO);
    let (out, err, status) = oslo(&[
        "--argc-eval",
        path.to_str().unwrap(),
        "--dry-run",
        "-t",
        "3",
        "prod",
    ]);
    assert_eq!(status, 0, "{err}");
    assert!(out.contains("argc_dry_run=1"), "{out}");
    assert!(out.contains("argc_tries=3"), "{out}");
    assert!(out.contains("argc_target=prod"), "{out}");
}

/// At a prompt there is no script, so it says what it is for rather than reporting that the shell
/// binary is not one.
#[test]
fn asked_at_a_prompt_it_says_what_it_is_for() {
    let (out, _, status) = oslo(&["-c", "argc --help"]);
    assert_eq!(status, 0);
    assert!(out.starts_with("usage: argc"), "{out}");
    assert!(out.contains("--argc-eval"), "{out}");
}
