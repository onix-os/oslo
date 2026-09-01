//! **A script, a pipe and a test see exactly what they saw before ariadne existed.**
//!
//! This is the safety net for the whole diagnostics change, and it was written before a single
//! site was converted. oslo is POSIX-first, and POSIX says what a shell writes to standard error:
//! a multi-line coloured report there would break `2>&1 | grep`, break every conformance suite,
//! and break scripts written before oslo existed.
//!
//! So the report is the *drawn face* of an error and the one-liner is its transport — the same
//! split `render_display` and `render_transport` are two functions for — and the thing that
//! decides between them is `isatty(2)` on stderr.
//!
//! Every runner here goes through `oslo -c`, whose stderr is a pipe. **Every string below is what
//! the shell printed before the change.** A converted site that alters one of them has broken the
//! rule, whatever it looks like on a terminal.
//!
//! # Why the strings are written out
//!
//! Not computed, not matched against a pattern. A test that asserted "stderr mentions the operand"
//! would pass on a report as happily as on a one-liner, which is exactly the regression it exists
//! to catch. The point is the bytes.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// One script, and the whole of the first line of stderr it produces.
///
/// The first line rather than all of stderr, because a few of these also print a usage block, and
/// this is a test about the *diagnostic*.
struct Plain {
    script: &'static str,
    stderr: &'static str,
}

/// The sites this change converts, and what they say to a pipe.
///
/// Grows a row per converted site. A row here is the contract: whatever the terminal gets, this is
/// what everything else gets.
const PLAIN: &[Plain] = &[
    // Phase 0 — the two worked examples.
    Plain {
        script: "kill -s NOPE 1",
        stderr: "oslo: kill: NOPE: invalid signal specification",
    },
    Plain {
        script: "df | cols nmae",
        stderr: "oslo: cols: nmae: no such column",
    },
    // Not converted, and here anyway: a diagnostic with nothing to point at must keep its
    // one-liner on a terminal too, so this row guards the decision as much as the output.
    Plain {
        script: "cd /nope",
        stderr: "oslo: cd: /nope: No such file or directory",
    },
    // The rest of the phase-1 population, pinned before it is touched.
    Plain {
        script: "kill %99",
        stderr: "oslo: kill: %99: no such job",
    },
    Plain {
        script: "printf '%d' abc",
        stderr: "oslo: printf: abc: invalid number",
    },
    Plain {
        script: "read -z x",
        stderr: "oslo: read: -z: invalid option",
    },
    Plain {
        script: "ulimit -Z",
        stderr: "oslo: ulimit: -Z: invalid option",
    },
    // Phase 1 — the builtins. One row per converted site; a row here is the contract.
    //
    // `options::invalid` serves six of these from one function, which is why `export`, `unset`,
    // `readonly`, `alias` and `unalias` all appear: converting the helper converted them, and a
    // row each is what says so.
    Plain {
        script: "export -z",
        stderr: "oslo: export: -z: invalid option",
    },
    Plain {
        script: "unset -z",
        stderr: "oslo: unset: -z: invalid option",
    },
    Plain {
        script: "readonly -z",
        stderr: "oslo: readonly: -z: invalid option",
    },
    Plain {
        script: "alias -z",
        stderr: "oslo: alias: -z: invalid option",
    },
    Plain {
        script: "unalias -z",
        stderr: "oslo: unalias: -z: invalid option",
    },
    Plain {
        script: "shopt -Z",
        stderr: "oslo: shopt: -Z: invalid option",
    },
    Plain {
        script: "hash -Z",
        stderr: "oslo: hash: -Z: invalid option",
    },
    Plain {
        script: "jobs -Z",
        stderr: "oslo: jobs: -Z: invalid option",
    },
    Plain {
        script: "umask -Z",
        stderr: "oslo: umask: -Z: invalid option",
    },
    Plain {
        script: "ulimit -Z",
        stderr: "oslo: ulimit: -Z: invalid option",
    },
    Plain {
        script: "command -Z",
        stderr: "oslo: command: -Z: invalid option",
    },
    Plain {
        script: "suspend -x",
        stderr: "oslo: suspend: -x: invalid option",
    },
    Plain {
        script: "times -p",
        stderr: "oslo: times: -p: invalid option",
    },
    Plain {
        script: "wait -z",
        stderr: "oslo: wait: -z: invalid option",
    },
    Plain {
        script: "mark -z",
        stderr: "oslo: mark: -z: not an option",
    },
    Plain {
        script: "builtin nosuch",
        stderr: "oslo: builtin: nosuch: not a shell builtin",
    },
    Plain {
        script: "chain nope",
        stderr: "oslo: chain: nope: unknown argument",
    },
    Plain {
        script: "ui nosuch",
        stderr: "oslo: ui: nosuch: not a widget",
    },
    Plain {
        script: "ui input --nope",
        stderr: "oslo: ui input: --nope: unknown option",
    },
    Plain {
        script: "while true; do break 0; done",
        stderr: "oslo: break: 0: loop count out of range",
    },
    Plain {
        script: "alias 'a b'=x",
        stderr: "oslo: alias: `a b': invalid alias name",
    },
    Plain {
        script: "shopt -s nosuch",
        stderr: "oslo: shopt: nosuch: invalid shell option name",
    },
    Plain {
        script: "shopt -o nosuch",
        stderr: "oslo: shopt: nosuch: invalid option name",
    },
    Plain {
        script: "unalias nosuch",
        stderr: "oslo: unalias: nosuch: not found",
    },
    Plain {
        script: "hash nosuch",
        stderr: "oslo: hash: nosuch: not found",
    },
    Plain {
        script: "disown -Z",
        stderr: "oslo: disown: -Z: invalid option",
    },
    Plain {
        script: "nav --nope",
        stderr: "oslo: nav: --nope: unknown option",
    },
    Plain {
        script: "kill -TERM %99",
        stderr: "oslo: kill: %99: no such job",
    },
    Plain {
        script: "export 2FOO=x",
        stderr: "oslo: export: `2FOO=x': not a valid identifier",
    },
    Plain {
        script: "unset a-b",
        stderr: "oslo: unset: `a-b': not a valid identifier",
    },
    Plain {
        script: "printf -v 2bad %s x",
        stderr: "oslo: printf: `2bad': not a valid identifier",
    },
    Plain {
        script: "read -t x y",
        stderr: "oslo: read: -t: x: invalid timeout specification",
    },
    Plain {
        script: "read -n",
        stderr: "oslo: read: -n: option requires an argument",
    },
];

/// stderr of `oslo -c script`, with `OSLO_DIAG` forced to `mode` when one is given.
fn stderr_of(script: &str, mode: Option<&str>) -> String {
    let mut command = Command::new(oslo_bin());
    command
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(mode) = mode {
        command.env("OSLO_DIAG", mode);
    }
    let output = command.output().expect("spawn oslo");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The rule, stated as plainly as it can be.
#[test]
fn a_pipe_sees_the_one_line_message() {
    for case in PLAIN {
        let stderr = stderr_of(case.script, None);
        let first = stderr.lines().next().unwrap_or("");
        assert_eq!(
            first, case.stderr,
            "`{}` changed what a pipe sees",
            case.script
        );
    }
}

/// Not one byte of a report may reach a pipe: no caret, no box, no colour, no `help:` line.
///
/// Checked separately from the string above because a report drawn *after* the one-liner would
/// leave the first line intact and still break every script that reads stderr.
#[test]
fn a_pipe_sees_no_report_at_all() {
    for case in PLAIN {
        let stderr = stderr_of(case.script, None);
        for mark in ['\u{1b}', '│', '─', '╭', '┌', '^'] {
            assert!(
                !stderr.contains(mark),
                "`{}` put {mark:?} on a pipe: {stderr:?}",
                case.script
            );
        }
        assert!(
            !stderr.contains("help:"),
            "`{}` put a help line on a pipe: {stderr:?}",
            case.script
        );
    }
}

/// **`OSLO_DIAG=never` is the escape hatch, and it has to work on a terminal too.**
///
/// A pipe already gets the plain form, so this can only be tested here for what it does *not*
/// change — but the row exists so the variable is wired from the first commit rather than added
/// once something needs turning off.
#[test]
fn never_is_the_same_as_a_pipe() {
    for case in PLAIN {
        assert_eq!(
            stderr_of(case.script, Some("never")),
            stderr_of(case.script, None),
            "`{}` under OSLO_DIAG=never",
            case.script
        );
    }
}

/// The exit status is a separate promise from the message, and just as easy to break: a site that
/// returns early to draw a report must still return the status it returned before.
#[test]
fn the_status_is_unchanged() {
    for (script, want) in [
        ("kill -s NOPE 1", 1),
        ("df | cols nmae", 2),
        ("cd /nope", 1),
        ("read -z x", 2),
    ] {
        let status = Command::new(oslo_bin())
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("spawn oslo")
            .code()
            .unwrap_or(-1);
        assert_eq!(status, want, "`{script}`");
    }
}
