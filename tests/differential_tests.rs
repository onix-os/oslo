//! Differential suite: every corpus script, run through rush and through bash.
//!
//! The audit that produced PLAN.md found one dominant failure mode — a plausible *wrong answer*
//! with exit status 0. No self-referential test can see that class of bug, because the expected
//! output has to come from somewhere other than the implementation under test. So it comes from
//! bash, which is the specification every `/bin/sh` script was actually written against.
//!
//! What is compared, and why only this much:
//!
//! * **stdout** — byte for byte, after the scratch directory is rewritten to `<TMP>` so the two
//!   shells' private working directories do not show up as a difference.
//! * **exit status** — exactly, with a signal death recorded as `128 + signo` the way a shell
//!   reports one.
//! * **stderr shape** — empty versus non-empty, nothing more. Two different shells will never
//!   agree on diagnostic wording and should not be forced to; what matters is that a shell that
//!   should complain does, and a shell that should stay quiet does.
//!
//! Every case runs with stdin on `/dev/null`, in its own scratch directory, under a wall-clock
//! timeout. rush has live hangs (`while read` never terminates), and an unguarded suite would
//! wedge CI instead of failing it — so a timeout is a first-class verdict here, not an accident.
//!
//! Corpus scripts declare their oracle on the first line: `# mode: posix` runs `bash --posix -c`
//! for POSIX semantics, `# mode: bash` runs plain `bash -c` for the bash extensions (arrays,
//! `[[ ]]`, `(( ))`, brace expansion) that rush also aims to support.

mod common;

#[path = "differential/expected_fail.rs"]
mod expected_fail;

use expected_fail::{EXPECTED_FAIL, KNOWN_DIVERGENT};

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Wall-clock budget per shell invocation. Generous: it exists to convert a hang into a failure,
/// not to measure performance.
fn timeout() -> Duration {
    let secs = std::env::var("RUSH_DIFF_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);
    Duration::from_secs(secs)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Oracle {
    Posix,
    Bash,
}

struct Case {
    name: String,
    oracle: Oracle,
    script: String,
}

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn load_corpus() -> Vec<Case> {
    let dir = corpus_dir();
    let mut cases: Vec<Case> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "sh"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            let script = fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {name}: {e}"));
            let oracle = match script.lines().next().unwrap_or_default().trim() {
                "# mode: posix" => Oracle::Posix,
                "# mode: bash" => Oracle::Bash,
                other => panic!(
                    "{name}: first line must be `# mode: posix` or `# mode: bash`, found {other:?}"
                ),
            };
            Case {
                name,
                oracle,
                script,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

struct Outcome {
    stdout: String,
    stderr_empty: bool,
    status: i32,
    timed_out: bool,
}

/// Run one shell on one script in a private scratch directory.
///
/// Output goes to files rather than pipes: a script that writes more than a pipe buffer would
/// otherwise block on a reader this function does not have, and look like a hang in the shell.
/// The capture files live *beside* the working directory, not in it, so `echo *` sees only what
/// the script itself created.
fn execute(program: &Path, args: &[&str], script: &str) -> io::Result<Outcome> {
    let root = tempfile::tempdir()?;
    let cwd = root.path().join("cwd");
    fs::create_dir(&cwd)?;
    let out_path = root.path().join("stdout");
    let err_path = root.path().join("stderr");

    let mut cmd = Command::new(program);
    cmd.args(args)
        .arg(script)
        .current_dir(&cwd)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&out_path)?)
        .stderr(fs::File::create(&err_path)?)
        // A user's $ENV or $BASH_ENV would be sourced by bash and ignored by rush, which is a
        // difference in the harness rather than in the shells.
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .env("LC_ALL", "C");

    let mut child = cmd.spawn()?;
    let deadline = Instant::now() + timeout();
    let (status, timed_out) = loop {
        match child.try_wait()? {
            Some(status) => break (status, false),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                break (child.wait()?, true);
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    };

    let stdout = String::from_utf8_lossy(&fs::read(&out_path)?).into_owned();
    let stderr_empty = fs::metadata(&err_path)?.len() == 0;

    // Both shells get a different scratch directory, so the path itself must not be a difference.
    let mut stdout = stdout.replace(&cwd.to_string_lossy().into_owned(), "<TMP>");
    if let Ok(real) = cwd.canonicalize() {
        stdout = stdout.replace(&real.to_string_lossy().into_owned(), "<TMP>");
    }

    Ok(Outcome {
        stdout,
        stderr_empty,
        status: status
            .code()
            .unwrap_or_else(|| 128 + status.signal().unwrap_or(0)),
        timed_out,
    })
}

enum Verdict {
    Match,
    /// rush never terminated. Always a defect, never an acceptable difference.
    Hung,
    Differ(String),
}

fn compare(case: &Case) -> Verdict {
    let rush = execute(&common::rush_bin(), &["-c"], &case.script).expect("spawn rush");
    let args: &[&str] = match case.oracle {
        Oracle::Posix => &["--posix", "-c"],
        Oracle::Bash => &["-c"],
    };
    let bash = execute(Path::new("bash"), args, &case.script)
        .expect("spawn bash — the differential suite needs bash on PATH as its oracle");

    if bash.timed_out {
        return Verdict::Differ(format!(
            "the oracle itself timed out; {} is not a usable corpus case",
            case.name
        ));
    }
    if rush.timed_out {
        return Verdict::Hung;
    }

    let mut report = String::new();
    if rush.stdout != bash.stdout {
        report.push_str("  stdout:\n");
        report.push_str(&diff_lines(&bash.stdout, &rush.stdout));
    }
    if rush.status != bash.status {
        report.push_str(&format!(
            "  status: bash {} vs rush {}\n",
            bash.status, rush.status
        ));
    }
    if rush.stderr_empty != bash.stderr_empty {
        report.push_str(&format!(
            "  stderr: bash {} vs rush {}\n",
            shape(bash.stderr_empty),
            shape(rush.stderr_empty)
        ));
    }

    if report.is_empty() {
        Verdict::Match
    } else {
        Verdict::Differ(report)
    }
}

fn shape(empty: bool) -> &'static str {
    if empty { "empty" } else { "non-empty" }
}

/// First few differing lines, aligned by position. Enough to identify the defect in a CI log
/// without turning the failure report into the corpus itself.
fn diff_lines(expected: &str, actual: &str) -> String {
    let exp: Vec<&str> = expected.lines().collect();
    let act: Vec<&str> = actual.lines().collect();
    let mut out = String::new();
    let mut shown = 0;
    for i in 0..exp.len().max(act.len()) {
        let (e, a) = (exp.get(i), act.get(i));
        if e == a {
            continue;
        }
        if shown == 6 {
            out.push_str("    …\n");
            break;
        }
        out.push_str(&format!(
            "    line {}: bash {:?} vs rush {:?}\n",
            i + 1,
            e.unwrap_or(&"<no line>"),
            a.unwrap_or(&"<no line>")
        ));
        shown += 1;
    }
    out
}

/// Run every case, spreading them over a few threads. Each case is its own process pair in its
/// own directory, so there is nothing to serialise.
fn run_all(cases: &[Case]) -> Vec<(String, Verdict)> {
    const THREADS: usize = 8;
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..THREADS)
            .map(|slot| {
                scope.spawn(move || {
                    cases
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % THREADS == slot)
                        .map(|(_, case)| (case.name.clone(), compare(case)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("worker thread"))
            .collect()
    })
}

/// The corpus uses constructs bash grew in 4.x (`${v^^}`, `&>`), so an ancient oracle would
/// report differences that say nothing about rush. macOS ships bash 3.2 for licensing reasons —
/// fail with the reason rather than with 40 mystery divergences.
fn assert_oracle_is_usable() {
    let out = Command::new("bash")
        .args(["-c", "echo ${BASH_VERSINFO[0]}"])
        .output()
        .expect("bash must be on PATH: it is this suite's oracle");
    let major: u32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0);
    assert!(
        major >= 4,
        "the oracle must be bash 4 or newer (found major version {major}); \
         on macOS install a current bash and put it ahead of /bin/bash on PATH"
    );
}

#[test]
fn corpus_matches_bash() {
    assert_oracle_is_usable();

    let cases = load_corpus();
    assert!(
        cases.len() >= 60,
        "the corpus is supposed to be substantial; found only {}",
        cases.len()
    );

    let expected: BTreeMap<&str, (&str, &str)> = EXPECTED_FAIL
        .iter()
        .map(|(file, id, why)| (*file, (*id, *why)))
        .collect();
    let divergent: BTreeSet<&str> = KNOWN_DIVERGENT.iter().map(|(file, _)| *file).collect();

    let cases: Vec<Case> = cases
        .into_iter()
        .filter(|c| !divergent.contains(c.name.as_str()))
        .collect();

    let mut unexpected_failures = Vec::new();
    let mut unexpected_passes = Vec::new();
    let mut matched = 0;
    let mut still_failing = 0;

    let mut results = run_all(&cases);
    results.sort_by(|a, b| a.0.cmp(&b.0));

    for (name, verdict) in results {
        let listed = expected.get(name.as_str());
        match (verdict, listed) {
            (Verdict::Match, None) => matched += 1,
            (Verdict::Match, Some((id, why))) => {
                unexpected_passes.push(format!(
                    "  {name} — listed as {id} (\"{why}\") but now matches bash: delete that line \
                     from tests/differential/expected_fail.rs"
                ));
            }
            (Verdict::Hung, Some(_)) | (Verdict::Differ(_), Some(_)) => still_failing += 1,
            (Verdict::Hung, None) => unexpected_failures
                .push(format!("  {name}\n    rush never terminated (timed out)\n")),
            (Verdict::Differ(detail), None) => {
                unexpected_failures.push(format!("  {name}\n{detail}"))
            }
        }
    }

    let mut report = String::new();
    if !unexpected_failures.is_empty() {
        report.push_str(&format!(
            "\n{} corpus case(s) diverge from bash and are not in EXPECTED_FAIL:\n\n{}",
            unexpected_failures.len(),
            unexpected_failures.join("\n")
        ));
    }
    if !unexpected_passes.is_empty() {
        report.push_str(&format!(
            "\n{} corpus case(s) pass while still listed as expected failures:\n\n{}\n",
            unexpected_passes.len(),
            unexpected_passes.join("\n")
        ));
    }
    assert!(
        report.is_empty(),
        "{report}\n({matched} matching, {still_failing} known-failing, \
         {} skipped as known-divergent)\n",
        divergent.len()
    );

    eprintln!(
        "differential corpus: {matched} matching bash, {still_failing} known-failing, \
         {} known-divergent",
        divergent.len()
    );
}

/// The lists are only a ratchet if they cannot drift away from the corpus they name.
#[test]
fn failure_lists_name_real_corpus_files() {
    let present: BTreeSet<String> = load_corpus().into_iter().map(|c| c.name).collect();

    let mut seen = BTreeSet::new();
    for (file, id, _) in EXPECTED_FAIL {
        assert!(
            present.contains(*file),
            "EXPECTED_FAIL names {file} ({id}), which is not in tests/corpus"
        );
        assert!(seen.insert(*file), "EXPECTED_FAIL lists {file} twice");
        assert!(
            id.starts_with('R') || *id == "UNFILED",
            "{file}: {id} is not a PLAN.md finding ID"
        );
    }
    for (file, why) in KNOWN_DIVERGENT {
        assert!(
            present.contains(*file),
            "KNOWN_DIVERGENT names {file}, which is not in tests/corpus"
        );
        assert!(!why.is_empty(), "{file}: a known divergence needs a reason");
        assert!(
            !seen.contains(*file),
            "{file} is in both EXPECTED_FAIL and KNOWN_DIVERGENT; it can only be one"
        );
    }
}
