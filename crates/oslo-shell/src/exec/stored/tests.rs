use super::*;

/// **The measurement that decided the design, as a test.** A script with a shebang runs from an
/// anonymous in-memory file: no path on disk, nothing to clean up, nothing another user can read.
#[test]
fn a_script_runs_without_ever_being_written_to_disk() {
    let body = "#!/bin/sh\nprintf 'ran %s\\n' \"$1\"\n";
    let fd = memory_file("demo", body.as_bytes()).expect("a memory file");
    let path = format!("/proc/self/fd/{}", fd.as_raw_fd());

    // What the descriptor points at is anonymous — not a name in any directory.
    let shown = std::fs::read_link(&path)
        .expect("readlink")
        .display()
        .to_string();
    assert!(shown.contains("memfd:oslo:demo"), "{shown}");
    assert!(shown.contains("(deleted)"), "no directory entry: {shown}");

    let out = std::process::Command::new(&path)
        .arg("here")
        .output()
        .expect("the kernel honours the shebang on a memfd");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ran here\n");
}

/// A shell script gets its own `$0` back, because `sh -c` takes the name as its next argument.
#[test]
fn a_shell_script_knows_its_own_name() {
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg("echo \"$0 saw $*\"")
        .arg("deploy")
        .arg("alpha")
        .output()
        .expect("sh");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "deploy saw alpha\n",
        "this is the repair `script` applies"
    );
}

/// The interpreters whose `$0` can be repaired, and the ones whose cannot.
#[test]
fn only_a_shell_gets_the_name_repair() {
    let shellish = |body: &str| {
        macros::shebang_interpreter(body)
            .filter(|i| matches!(i.as_str(), "sh" | "bash" | "dash" | "ksh" | "zsh" | "oslo"))
    };
    assert_eq!(shellish("#!/bin/sh\n").as_deref(), Some("sh"));
    assert_eq!(shellish("#!/usr/bin/env bash\n").as_deref(), Some("bash"));
    assert_eq!(shellish("#!/usr/bin/env python3\n"), None, "no -c trick");
    assert_eq!(shellish("no shebang\n"), None);
}

/// The body is what was stored, byte for byte — a script is not text the shell gets to tidy.
#[test]
fn what_is_written_is_what_comes_back() {
    let body = "#!/bin/sh\n# a comment with\ttabs\nprintf 'x'\n";
    let fd = memory_file("exact", body.as_bytes()).expect("a memory file");
    let read = std::fs::read_to_string(format!("/proc/self/fd/{}", fd.as_raw_fd())).expect("read");
    assert_eq!(read, body);
}
