//! `kill`: signal delivery that resolves the spec before it sends anything.
//!
//! The previous implementation seeded `sig` with SIGTERM and kept that default when the name
//! failed to parse — and nix only accepts the `SIG`-prefixed spelling, so `-HUP`, `-USR1` and
//! `-stop` all quietly sent TERM, and `kill -0 $pid`, the canonical "is it still alive?" probe,
//! *terminated the process it was asked about*. The order here is deliberate: the spec is
//! resolved to a number first, and an unresolvable spec returns before a single signal is sent.

use super::signals;
use crate::env::scope::Environment;
use crate::error::Result;
use nix::errno::Errno;

/// What the argument scan produced. `spec` is unresolved on purpose — parsing it is the caller's
/// job, so that "no operands" and "bad signal" stay two distinguishable failures.
struct Invocation<'a> {
    list: bool,
    spec: Option<&'a str>,
    operands: &'a [String],
}

pub fn builtin_kill(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(inv) = parse_args(args.get(1..).unwrap_or(&[])) else {
        return Ok(usage());
    };

    if inv.list {
        return Ok(list(inv.operands));
    }

    // Default only when nothing was asked for. A spec that was given and did not parse is an
    // error, never a silent fallback to TERM.
    let spec = inv.spec.unwrap_or("TERM");
    let Some(signum) = signals::parse_spec(spec) else {
        eprintln!("rush: kill: {spec}: invalid signal specification");
        return Ok(1);
    };

    if inv.operands.is_empty() {
        return Ok(usage());
    }

    // bash reports success when at least one operand was signalled, so a dead pid in a list does
    // not mask the live ones. Nothing here stops at the first failure.
    let mut any_succeeded = false;
    for operand in inv.operands {
        match operand.parse::<i32>() {
            Ok(pid) => match send(pid, signum) {
                Ok(()) => any_succeeded = true,
                Err(e) => eprintln!("rush: kill: ({operand}) - {}", e.desc()),
            },
            Err(_) => eprintln!("rush: kill: `{operand}': not a pid or valid job spec"),
        }
    }

    Ok(if any_succeeded { 0 } else { 1 })
}

/// Split options from operands. Returns `None` when the invocation is malformed enough that the
/// only useful answer is the usage line.
fn parse_args(rest: &[String]) -> Option<Invocation<'_>> {
    let mut list = false;
    let mut spec: Option<&str> = None;
    let mut idx = 0;

    while idx < rest.len() {
        let arg = rest[idx].as_str();
        if arg == "--" {
            idx += 1;
            break;
        }
        // Once a signal is chosen, a leading `-` belongs to an operand: `kill -TERM -1234`
        // signals process group 1234.
        if spec.is_some() || !arg.starts_with('-') || arg == "-" {
            break;
        }
        match arg {
            "-l" | "-L" => {
                list = true;
                idx += 1;
            }
            "-s" | "-n" => {
                spec = Some(rest.get(idx + 1)?.as_str());
                idx += 2;
            }
            _ => {
                spec = Some(&arg[1..]);
                idx += 1;
            }
        }
    }

    let operands = &rest[idx..];
    if !list && spec.is_none() && operands.is_empty() {
        return None;
    }
    Some(Invocation {
        list,
        spec,
        operands,
    })
}

fn usage() -> i32 {
    eprintln!(
        "rush: kill: usage: kill [-s sigspec | -n signum | -sigspec] pid | jobspec ... or kill -l [sigspec]"
    );
    2
}

/// `kill -l`: the whole table, or the other half of each spec given.
fn list(specs: &[String]) -> i32 {
    if specs.is_empty() {
        let names: Vec<String> = signals::all().into_iter().map(|(_, name)| name).collect();
        println!("{}", names.join(" "));
        return 0;
    }

    let mut status = 0;
    for spec in specs {
        match describe(spec) {
            Some(line) => println!("{line}"),
            None => {
                eprintln!("rush: kill: {spec}: invalid signal specification");
                status = 1;
            }
        }
    }
    status
}

/// A number becomes a name, a name becomes a number.
fn describe(spec: &str) -> Option<String> {
    let Ok(num) = spec.parse::<i32>() else {
        return signals::number_from_name(spec).map(|n| n.to_string());
    };
    if num == 0 {
        // Shells name signal 0 EXIT here: `kill -l $?` on a trap-driven status should read back.
        return Some("EXIT".to_string());
    }
    // `kill -l $?` on a signalled child is given 128 + signo, so fall back to the wait-status
    // encoding rather than calling a perfectly meaningful status invalid.
    signals::name_from_number(num).or_else(|| signals::name_from_number(num - 128))
}

/// `libc::kill` rather than nix's wrapper: nix's `Signal` enum has no realtime signals and so
/// cannot express `kill -RTMIN+3`. Signal 0 passes straight through — that is the existence
/// probe, and the kernel delivers nothing for it.
fn send(pid: i32, signum: i32) -> std::result::Result<(), Errno> {
    // SAFETY: kill(2) takes two scalars, reads no memory owned by this process and cannot
    // invalidate any Rust invariant. Failure is reported through errno, which is what
    // `Errno::result` reads.
    let res = unsafe { nix::libc::kill(pid, signum) };
    Errno::result(res).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// The regression that named this finding: an unparseable spec must send nothing at all.
    /// Signal 0 against our own pid is the only send this suite performs, precisely because it
    /// is the one that must be inert.
    #[test]
    fn a_bad_signal_spec_sends_nothing_and_fails() {
        let mut env = Environment::new();
        let self_pid = std::process::id().to_string();
        for bad in ["-NOSUCHSIG", "-99", "-sig"] {
            assert_eq!(
                builtin_kill(&mut env, &words(&["kill", bad, &self_pid])).unwrap(),
                1,
                "{bad} should be an invalid signal specification"
            );
        }
        assert_eq!(
            builtin_kill(&mut env, &words(&["kill", "-s", "NOSUCHSIG", &self_pid])).unwrap(),
            1
        );
    }

    #[test]
    fn signal_zero_probes_a_live_process() {
        let mut env = Environment::new();
        let self_pid = std::process::id().to_string();
        assert_eq!(
            builtin_kill(&mut env, &words(&["kill", "-0", &self_pid])).unwrap(),
            0
        );
        // Still running to make the assertion, which is the whole point.
        assert_eq!(
            builtin_kill(&mut env, &words(&["kill", "-s", "0", &self_pid])).unwrap(),
            0
        );
    }

    #[test]
    fn a_non_numeric_operand_is_diagnosed() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_kill(&mut env, &words(&["kill", "-0", "abc"])).unwrap(),
            1
        );
        assert_eq!(builtin_kill(&mut env, &words(&["kill", "abc"])).unwrap(), 1);
    }

    #[test]
    fn missing_operands_are_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(builtin_kill(&mut env, &words(&["kill"])).unwrap(), 2);
        assert_eq!(
            builtin_kill(&mut env, &words(&["kill", "-TERM"])).unwrap(),
            2
        );
        assert_eq!(builtin_kill(&mut env, &words(&["kill", "-s"])).unwrap(), 2);
    }

    #[test]
    fn list_translates_both_directions() {
        assert_eq!(describe("9").as_deref(), Some("KILL"));
        assert_eq!(describe("KILL").as_deref(), Some("9"));
        assert_eq!(describe("SIGKILL").as_deref(), Some("9"));
        assert_eq!(describe("0").as_deref(), Some("EXIT"));
        assert_eq!(describe("137").as_deref(), Some("KILL"));
        assert_eq!(describe("NOSUCHSIG"), None);
    }

    /// Options must be recognised where a shell puts them, and stop where operands begin.
    #[test]
    fn argument_scan_separates_options_from_pids() {
        let args = words(&["-s", "HUP", "123", "456"]);
        let inv = parse_args(&args).expect("parses");
        assert_eq!(inv.spec, Some("HUP"));
        assert_eq!(inv.operands, &args[2..]);

        // A negative operand after a signal is a process group, not another option.
        let args = words(&["-TERM", "-1234"]);
        let inv = parse_args(&args).expect("parses");
        assert_eq!(inv.spec, Some("TERM"));
        assert_eq!(inv.operands.len(), 1);

        // `--` ends the options: everything after it is an operand.
        let args = words(&["--", "-1234"]);
        let inv = parse_args(&args).expect("parses");
        assert!(inv.spec.is_none());
        assert_eq!(inv.operands.len(), 1);

        let args = words(&["-l"]);
        assert!(parse_args(&args).expect("parses").list);
    }
}
