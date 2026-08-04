//! oslo's own tools, reached by the name it was called by.
//!
//! # Why not `oslo config`
//!
//! Because that already means something. POSIX defines the shell's synopsis as
//! `sh [options] [command_file [argument...]]` — the first operand **is a script path**, and
//! neither bash nor dash reserves a single word in that slot. `oslo config` means "run the script
//! named `config`", and a shell that decided otherwise would break the idiom on a `/bin/sh`.
//!
//! The case that settles it is the shebang. A script beginning `#!/bin/oslo` is executed by the
//! kernel as `execve("/bin/oslo", ["/bin/oslo", "./config"])` — argv identical, byte for byte, to
//! somebody typing `oslo config`. There is nothing to tell them apart, so a subcommand named
//! `config` would silently swallow every `#!/bin/oslo` script called `config`. On a machine where
//! oslo *is* `/bin/sh`, that is every script on it.
//!
//! # argv[0] instead
//!
//! busybox's answer, and it works because `argv[0]` is a slot the shell never reads as a script
//! path. One binary, extra names:
//!
//! ```text
//! ln -s /usr/bin/oslo /usr/bin/oslo-config
//! ```
//!
//! `oslo-config` runs the tool; `oslo` and `sh` are the shell, untouched. The two can never
//! collide because they are never reached through the same `argv[0]`. Verified against busybox
//! itself, which has hundreds of applets and reserves *none* of them in `sh` mode: `busybox sh
//! sync` runs a script named `sync`, not the `sync` applet.
//!
//! The names are prefixed `oslo-` because oslo is not replacing coreutils — a bare `config` on
//! `$PATH` would shadow somebody else's program. busybox drops the prefix precisely because
//! shadowing coreutils is its job.

/// One tool.
pub struct Tool {
    /// The part after `oslo-`.
    pub name: &'static str,
    /// One line, for the help.
    pub about: &'static str,
}

/// Every tool oslo knows how to be.
///
/// The single list: the dispatcher and the help both read it, so a tool cannot be reachable and
/// undocumented, or listed and unreachable.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "config",
        about: "inspect and edit the Lua configuration",
    },
    Tool {
        name: "profile",
        about: "list, create and switch history profiles",
    },
    Tool {
        name: "history",
        about: "search, export and prune the command history",
    },
    Tool {
        name: "direnv",
        about: "manage per-directory environments",
    },
    Tool {
        name: "hook",
        about: "list and test the shell hooks",
    },
];

/// The tool a first *operand* names, if it safely names one.
///
/// This is what makes `oslo history` work without taking the operand slot away from scripts. Three
/// conditions, and all of them must hold:
///
/// 1. **No `/` in it.** Every shebang produces a slashed path — `./config` when run from the
///    current directory, the full path when found on `$PATH` — so a bare word can only have been
///    typed by a person. Verified against the kernel rather than assumed.
/// 2. **No file of that name exists.** A script always wins. `oslo config` next to a `./config`
///    runs the script, exactly as it does today.
/// 3. It is one of [`TOOLS`].
///
/// Condition 2 is what makes this safe rather than merely unlikely to bite. oslo does not search
/// `$PATH` for a script operand, so if no such file exists the alternative was not "run something
/// else" — it was `oslo: config: No such file or directory`. Nothing that works today can change
/// meaning; only an error becomes useful.
///
/// The escape hatches, for the day somebody has a script named `hook`: `oslo ./hook` and
/// `oslo -- hook` both say "this is a path" and are honoured.
pub fn as_operand(word: &str) -> Option<&'static Tool> {
    as_operand_when(word, |word| std::path::Path::new(word).exists())
}

/// [`as_operand`], with the filesystem passed in.
///
/// Split out **so the tests never touch the process's working directory.** Proving "a real file
/// wins" by `chdir`-ing into a temporary directory would work exactly once: `cwd` is process-wide,
/// libtest runs tests on threads, and a sibling resolving a relative path mid-`chdir` sees the
/// wrong one. The same in-process global-state trap as `environ`, which has caused three flaky
/// tests in this codebase already.
fn as_operand_when(word: &str, exists: impl Fn(&str) -> bool) -> Option<&'static Tool> {
    if word.contains('/') {
        return None;
    }
    if exists(word) {
        return None;
    }
    from_name(word)
}

/// The tool with exactly this name.
pub fn from_name(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

/// The tool `called_as` names, if it names one.
///
/// Takes the whole `argv[0]` and reduces it: `/usr/bin/oslo-config` and `oslo-config` are the same
/// request. A leading `-`, which is how a login shell is invoked, cannot survive the `oslo-`
/// prefix test and so is never mistaken for a tool.
pub fn from_argv0(called_as: &str) -> Option<&'static Tool> {
    let base = called_as.rsplit('/').next()?;
    from_name(base.strip_prefix("oslo-")?)
}

/// Whether `oslo-<name>` on `$PATH` is a signpost back to *this* binary.
///
/// **Resolved and compared, not merely tested for existence.** A different program of that name
/// would answer to it instead, and reporting that as available would be a lie the user only finds
/// out by running it.
///
/// `$PATH` rather than the directory beside the binary, because the question being asked is "if I
/// type `oslo-config`, will it work?" — which is a `$PATH` question, and stays right for a distro
/// that puts the real binary in `/usr/lib/oslo/` with the signposts in `/usr/bin/`.
pub fn linked(name: &str) -> bool {
    let Ok(me) = std::fs::canonicalize("/proc/self/exe") else {
        return false;
    };
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|dir| std::fs::canonicalize(dir.join(format!("oslo-{name}"))).is_ok_and(|f| f == me))
}

/// Run a tool. The status is the process's.
///
/// Every tool answers `--help` and nothing else yet — the sub-subcommands are still to be written.
/// A stub that *says* it is a stub beats one that accepts arguments and ignores them: this way a
/// script built against a tool that does not do its job yet fails now rather than silently.
pub fn run(tool: &'static Tool, args: &[String]) -> i32 {
    let paint = crate::cli::help::Paint::detect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", help(tool, paint));
        return 0;
    }
    if let Some(unknown) = args.first() {
        eprint!("{}", help(tool, paint));
        eprintln!("\noslo-{}: {unknown:?}: no such subcommand", tool.name);
        return 2;
    }
    print!("{}", help(tool, paint));
    0
}

/// A tool's own help.
fn help(tool: &'static Tool, paint: crate::cli::help::Paint) -> String {
    let name = format!("oslo-{}", tool.name);
    format!(
        "{}\n  {} {}\n\n{}\n  {}\n",
        paint.head("USAGE"),
        paint.key(&name),
        paint.slot("<subcommand> [...]"),
        paint.head("SUBCOMMANDS"),
        paint.dim("none yet — this tool is not implemented"),
    )
}

/// Where this binary really lives, for the hint that tells you how to make a signpost.
///
/// The resolved path rather than a hardcoded `/usr/bin/oslo`, so the `ln -s` line printed is one
/// that can be pasted — which is the whole difference between a hint and a chore.
pub fn own_path() -> String {
    std::fs::canonicalize("/proc/self/exe")
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "oslo".to_string())
}

#[cfg(test)]
#[path = "tools/tests.rs"]
mod tests;
