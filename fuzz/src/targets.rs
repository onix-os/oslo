//! The three harness bodies, one per fuzz target.
//!
//! Each takes raw fuzzer bytes and returns `()`. Errors are expected and ignored — a parser
//! refusing malformed input is the parser working. What these look for is the other outcomes: a
//! panic, an abort from a debug-build overflow, a stack exhaustion from unbounded recursion, or a
//! non-terminating loop.

use rush::Environment;
use rush::lexer::{Lexer, Token, parse_single_word};
use std::sync::{Mutex, MutexGuard, Once};

use crate::{MAX_EXPR, MAX_SCRIPT, MAX_WORD, opens_command_substitution, text};

/// Parse a whole script. No part of the AST is executed.
pub fn parse_script(data: &[u8]) {
    let Some(source) = text(data, MAX_SCRIPT) else {
        return;
    };
    let _ = rush::parse_bash_script(&source);
}

/// Run the word lexer and its token scanner over one input.
///
/// Both halves are driven, because they fail differently: the scanner owns operator recognition
/// and the reserved-word table, while `parse_single_word` owns quoting, `$` expansion shapes and
/// ANSI-C escapes. Lexing builds an AST; it never runs a command substitution it finds.
pub fn lex_word(data: &[u8]) {
    let Some(source) = text(data, MAX_WORD) else {
        return;
    };

    // Every token but `Eof` consumes at least one byte, so a scanner that yields more than
    // `len + 1` of them has stopped advancing. Checking that here turns a hang — which a fuzzer
    // can only report as a timeout, minutes later, with no useful backtrace — into a panic
    // pointing at the input that caused it. It earned its keep in the first ninety seconds of
    // fuzzing: see `fuzz/known/README.md`.
    let budget = source.len() + 1;
    let mut lexer = Lexer::new(&source);
    for produced in 0..=budget {
        match lexer.next() {
            Ok(Token::Eof) | Err(_) => break,
            Ok(_) => {
                assert!(
                    produced < budget,
                    "lexer produced more than {budget} tokens for {source:?}: not advancing"
                );
            }
        }
    }

    let _ = parse_single_word(&source);
}

/// Evaluate one arithmetic expression against a fixed environment.
///
/// This is PLAN.md R3.5's target: the Round 1 overflow guards (`i64::MIN / -1`, wrapping shifts,
/// division by zero) and the Round 3 lexer/parser/eval split have to hold together, and neither a
/// unit test nor the differential corpus will find the operand combination that breaks them.
///
/// Inputs that would fork a command are dropped, not evaluated — see
/// [`crate::opens_command_substitution`].
pub fn eval_arith(data: &[u8]) {
    let Some(expr) = text(data, MAX_EXPR) else {
        return;
    };
    if opens_command_substitution(&expr) {
        return;
    }
    let mut env = fuzz_env();
    let _ = rush::expand::arithmetic::eval_arithmetic(&mut env, &expr);
}

/// A shell environment with a fixed, interesting set of variables.
///
/// The variables are chosen to reach the guards rather than to look realistic: the two `i64`
/// extremes so `$((big + big))` and `$((neg / -1))` are one mutation away, a value that is itself
/// an expression, and a self-referential name that must hit the resolve-depth ceiling instead of
/// the stack.
pub fn fuzz_env() -> Environment {
    let _guard = environment_lock();

    let mut env = Environment::new();
    for (name, value) in [
        ("x", "1"),
        ("y", "-2"),
        ("big", "9223372036854775807"),
        ("neg", "-9223372036854775808"),
        ("hex", "0x7fffffffffffffff"),
        ("expr", "1+1"),
        ("cycle", "cycle"),
        ("word", "not-a-number"),
        ("empty", ""),
    ] {
        env.set_var(name, value, false);
    }
    env.set_positional(vec!["1".to_string(), "-2".to_string(), "3".to_string()]);
    env
}

/// Serialise every touch of the process environment, and clear it before the first one.
///
/// Two problems, one lock.
///
/// *Determinism:* `Environment::new` copies `std::env::vars()` in, so an arithmetic result would
/// otherwise depend on the machine that ran the fuzzer — and a crash that only reproduces under
/// one developer's exported variables is a crash nobody can act on. The inherited environment is
/// removed once, before the first `Environment` exists, which is what makes a saved artifact
/// replayable anywhere.
///
/// *Safety:* `remove_var` is not thread-safe against a concurrent read, and `Environment::new`
/// reads. libFuzzer drives its target on one thread, but `cargo test` does not, so the clear and
/// every construction that follows it happen under the same mutex rather than on trust.
fn environment_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    static CLEARED: Once = Once::new();

    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    CLEARED.call_once(|| {
        for (name, _) in std::env::vars() {
            // SAFETY: the lock is held, and it is the only path in this crate that reads or
            // writes the process environment, so no concurrent access is possible.
            unsafe { std::env::remove_var(name) };
        }
    });
    guard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn repo_dir(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
    }

    fn files_under(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(files_under(&path));
            } else if path.is_file() {
                out.push(path);
            }
        }
        out.sort();
        out
    }

    /// Replay every committed input through every target.
    ///
    /// This is the stable-toolchain half of the harness: no libFuzzer, no sanitizer, no nightly.
    /// It proves the targets still compile and still survive the corpus, which is exactly the
    /// regression that a fuzz suite quietly loses when nobody runs it for a month.
    ///
    /// The replay is watchdogged, because the first bug this harness found was a hang and not a
    /// panic. Without the watchdog a single bad seed would wedge CI for its whole job timeout and
    /// report nothing about which input did it.
    #[test]
    fn committed_corpus_survives_every_target() {
        let mut inputs: Vec<PathBuf> = files_under(&repo_dir("tests/corpus"));
        assert!(
            inputs.len() > 300,
            "expected the shell corpus at tests/corpus to seed this suite, found {}",
            inputs.len()
        );
        inputs.extend(files_under(&repo_dir("fuzz/seeds")));

        replay_under_watchdog(inputs);
    }

    /// How long one input may take before the replay calls it a hang.
    ///
    /// Generous: the whole 400-input replay takes under a second, so a single input sitting here
    /// for fifteen has stopped making progress rather than being slow. Not longer, because the
    /// hang this caught also allocates while it spins — waiting it out costs memory, not just
    /// wall clock.
    const INPUT_DEADLINE: Duration = Duration::from_secs(15);

    /// Run every input through every target on a worker thread, with the test thread watching it.
    ///
    /// A hung target cannot be interrupted — Rust has no way to kill a thread — so the watchdog
    /// names the input and ends the process. That loses the other tests' results, which is the
    /// right trade: an infinite loop in a parser is the most severe thing in this directory, and
    /// it has to be reported as a failure with an input attached rather than as a job timeout.
    fn replay_under_watchdog(inputs: Vec<PathBuf>) {
        let progress = Arc::new(AtomicUsize::new(0));
        let current: Arc<Mutex<PathBuf>> = Arc::new(Mutex::new(PathBuf::new()));

        let worker = {
            let progress = Arc::clone(&progress);
            let current = Arc::clone(&current);
            let inputs = inputs.clone();
            std::thread::spawn(move || {
                for path in inputs {
                    *current.lock().expect("watchdog mutex") = path.clone();
                    let data = fs::read(&path).expect("corpus file is readable");
                    parse_script(&data);
                    lex_word(&data);
                    eval_arith(&data);
                    progress.fetch_add(1, Ordering::SeqCst);
                }
            })
        };

        let total = inputs.len();
        let mut last_seen = 0;
        let mut stalled = Duration::ZERO;
        while !worker.is_finished() {
            std::thread::sleep(Duration::from_millis(100));
            let done = progress.load(Ordering::SeqCst);
            if done == last_seen {
                stalled += Duration::from_millis(100);
            } else {
                last_seen = done;
                stalled = Duration::ZERO;
            }
            if stalled >= INPUT_DEADLINE {
                let path = current.lock().expect("watchdog mutex").clone();
                // Written to the raw handle, not `eprintln!`: the test harness captures the
                // macros and replays them when a test *returns*, and this one never will.
                let _ = write!(
                    std::io::stderr(),
                    "\nhang: {} did not finish within {INPUT_DEADLINE:?} ({done}/{total} inputs \
                     replayed). A parser that does not terminate on this input is the bug; the \
                     input is the reproducer.\n",
                    path.display()
                );
                let _ = std::io::stderr().flush();
                std::process::exit(101);
            }
        }
        worker.join().expect("replay worker panicked");
    }

    /// Every input under `fuzz/known/` must still fail.
    ///
    /// Same discipline as `tests/differential/expected_fail.rs`: a bug the fuzzer found and nobody
    /// has fixed yet is recorded, not forgotten, and the record is what fails the day the bug goes
    /// away. Without this the reproducer would sit in a directory for a year and no one would
    /// notice it had become obsolete — or, worse, would notice only after it regressed again.
    #[test]
    fn known_findings_are_still_open() {
        let previous = std::panic::take_hook();
        // The panic these inputs cause is the expected result; printing its backtrace would make
        // a passing test look like a failing one.
        std::panic::set_hook(Box::new(|_| {}));

        let mut results = Vec::new();
        for path in files_under(&repo_dir("fuzz/known")) {
            if path.extension().is_some_and(|ext| ext == "md") {
                continue;
            }
            let data = fs::read(&path).expect("known-finding file is readable");
            results.push((path, still_reproduces(data)));
        }

        std::panic::set_hook(previous);

        for (path, still_fails) in results {
            assert!(
                still_fails,
                "{} no longer reproduces: the bug it documents is fixed. Delete the file and its \
                 entry in fuzz/known/README.md, and drop the target from FUZZ_KNOWN_OPEN in \
                 .github/workflows/fuzz.yml.",
                path.display()
            );
        }
    }

    /// How long a known finding gets to prove it is still a finding.
    ///
    /// `fuzz/known/` is empty as of Round 11 — both findings it held are fixed and their inputs
    /// moved to `fuzz/seeds/` — so this bounds nothing today and exists for the next entry. Both
    /// of the two it did hold were non-termination in one form or another, which is why "did not
    /// finish" counts as a reproduction here rather than as an inconclusive result. Two seconds
    /// against a normal input's fraction of a millisecond leaves the slowest CI runner
    /// unambiguous.
    const KNOWN_DEADLINE: Duration = Duration::from_secs(2);

    /// Does this input still panic, or still fail to terminate?
    ///
    /// Both count. The first finding was a panic (an advancement assertion) and the second is a
    /// parse that never returns, and a record of open bugs that could only hold one of those
    /// shapes would have quietly dropped the more serious one.
    ///
    /// A worker thread that never comes back is left running: Rust cannot cancel a thread, and the
    /// alternative — waiting for it — is exactly the hang being detected. It burns a core until
    /// the test binary exits, which is seconds away and is the cheapest correct option available.
    fn still_reproduces(data: Vec<u8>) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let outcome = std::panic::catch_unwind(|| {
                parse_script(&data);
                lex_word(&data);
                eval_arith(&data);
            });
            let _ = tx.send(outcome.is_err());
        });

        // A timeout is a reproduction, not an inconclusive answer: not terminating is the bug.
        rx.recv_timeout(KNOWN_DEADLINE).unwrap_or(true)
    }

    #[test]
    fn arithmetic_guards_answer_instead_of_aborting() {
        // These are the shapes R3.5 names. A debug build must not abort on any of them; whether
        // the answer is a value or an error is the shell's business, not the harness's.
        for expr in [
            "9223372036854775807 + 1",
            "-9223372036854775808 / -1",
            "-9223372036854775808 % -1",
            "big + big",
            "neg / -1",
            "1 << 64",
            "1 / 0",
            "cycle + 1",
            "expr * 2",
            "$1 - $2",
        ] {
            eval_arith(expr.as_bytes());
        }
    }

    #[test]
    fn deep_nesting_is_refused_not_overflowed() {
        // The nesting pre-check exists because brush's recursive descent overflows the stack
        // before any rush code can report it. A fuzzer finds this input in seconds.
        for depth in [50, 200, 5_000] {
            parse_script(format!("{}{}", "(".repeat(depth), ")".repeat(depth)).as_bytes());
            parse_script(format!("{}true{}", "{ ".repeat(depth), "; }".repeat(depth)).as_bytes());
            eval_arith(format!("{}1{}", "(".repeat(depth), ")".repeat(depth)).as_bytes());
        }
    }

    #[test]
    fn command_substitution_never_reaches_the_evaluator() {
        // If this ever regressed, the fuzzer would be running commands a mutator invented.
        let marker = repo_dir("fuzz/target/fuzz-must-never-run");
        let _ = std::fs::remove_file(&marker);
        let expr = format!("$(touch {})", marker.display());
        eval_arith(expr.as_bytes());
        assert!(!marker.exists(), "the arithmetic target executed a command");
    }
}
