//! `declare`, and the attributes this shell refuses rather than pretends to have.
//!
//! The unit tests beside the builtin already cover the decision. These go through the real binary
//! because the failure that matters is what a *script* sees: the status it can branch on and the
//! diagnostic a person reads, neither of which a direct call to the builtin function exercises.

mod common;

use common::oslo_bin;
use std::process::Command;

struct Ran {
    stdout: String,
    stderr: String,
    status: i32,
}

fn oslo(script: &str) -> Ran {
    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg(script)
        .output()
        .expect("spawn oslo");
    Ran {
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        status: out.status.code().unwrap_or(-1),
    }
}

/// **Refused loudly, and named.** bash accepts `declare -A`; oslo has no second value shape, so it
/// says so and exits 2.
///
/// The alternative — accepting it and building an *indexed* array — is the failure this shell is
/// audited against: every key would land on element 0 (see `tests/corpus/array_element_assignment.sh`),
/// the last write would win, and nothing on screen would look wrong. A diagnostic beats a plausible
/// wrong answer with status 0.
#[test]
fn an_associative_array_is_refused_rather_than_faked() {
    let ran = oslo("declare -A m");
    assert_eq!(
        ran.status, 2,
        "stdout {:?} stderr {:?}",
        ran.stdout, ran.stderr
    );
    assert!(
        ran.stderr.contains("associative arrays are not supported"),
        "the reason was not named: {:?}",
        ran.stderr
    );

    // And nothing was created under that name, so a later write cannot look like it worked.
    let ran = oslo("declare -A m 2>/dev/null; m[a]=1; m[b]=2; echo \"${m[a]}|${m[b]}|${#m[@]}\"");
    assert_eq!(
        ran.stdout.trim(),
        "2|2|1",
        "the subscript stopped being arithmetic, which is how a fake -A would look"
    );
}

/// `typeset` is the same builtin under its other name, so it refuses identically — a script that
/// reaches for the older spelling must not get a different answer.
#[test]
fn typeset_refuses_the_same_attribute() {
    let ran = oslo("typeset -A m");
    assert_eq!(ran.status, 2);
    assert!(ran.stderr.contains("associative arrays are not supported"));
}

/// The other attributes with no representation here. Grouped in one test because the rule is one
/// rule: an attribute that cannot be honoured is refused, never downgraded to a plain scalar.
#[test]
fn every_unrepresentable_attribute_is_refused() {
    for flag in ["-i", "-l", "-u", "-n"] {
        let ran = oslo(&format!("declare {flag} v"));
        assert_eq!(ran.status, 2, "{flag} was accepted");
        assert!(
            ran.stderr.contains("attribute not supported"),
            "{flag}: {:?}",
            ran.stderr
        );
        // Nothing is left behind under the name, in either shape.
        let after = oslo(&format!(
            "declare {flag} v 2>/dev/null; echo \"[${{v-unset}}][${{#v[@]}}]\""
        ));
        assert_eq!(after.stdout.trim(), "[unset][0]", "{flag} left something");
    }
}

/// The ones oslo *does* have keep working, so the refusals above are a short list rather than a
/// general shrug at `declare`.
#[test]
fn the_attributes_this_shell_has_still_work() {
    assert_eq!(
        oslo("declare -a a; a[2]=x; echo ${#a[@]}").stdout.trim(),
        "1"
    );
    assert_eq!(oslo("declare -r r=1; echo $r").stdout.trim(), "1");
    assert_eq!(
        oslo("declare -x e=1; sh -c 'echo $e'").stdout.trim(),
        "1",
        "-x did not export"
    );
    assert_eq!(
        oslo("f() { declare -g g=out; }; f; echo $g").stdout.trim(),
        "out"
    );
}
