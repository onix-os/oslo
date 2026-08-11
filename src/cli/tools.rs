//! oslo's own tools: `oslo config`, `oslo history`, and the rest.
//!
//! # Sharing the operand slot with scripts
//!
//! POSIX defines the shell's synopsis as `sh [options] [command_file [argument...]]` — the first
//! operand **is a script path**, and neither bash nor dash reserves a single word in that slot. So
//! a tool name has to be read there without ever taking an invocation a script could have wanted.
//!
//! [`as_operand`] is that rule, and it holds because of one measured fact: **every shebang
//! produces a slashed argv[1]**. A `#!/bin/oslo` script is executed by the kernel as
//! `execve("/bin/oslo", ["/bin/oslo", "./config"])` when run from the current directory, and with
//! the full path when found on `$PATH`. A bare `config` in that slot can therefore only have been
//! typed by a person — which is what leaves the word free to mean something.
//!
//! The second condition does the rest: a file of that name always wins. oslo does not search
//! `$PATH` for a script operand, so when no such file exists the alternative was never "run
//! something else", it was `No such file or directory`. An error becomes useful; nothing that
//! worked changes meaning.

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
    #[cfg(feature = "direnv")]
    Tool {
        name: "direnv",
        about: "manage per-directory environments",
    },
    Tool {
        name: "hook",
        about: "list and test the shell hooks",
    },
    // **A tool as well as a builtin, and the two are not redundant.** The builtin is what a person
    // types at an oslo prompt; this is what anything *else* asks — a prompt segment, a status bar,
    // a script — and those reach oslo through `io.popen` or `sh -c`, where a builtin does not
    // exist and the name resolves to whatever is on `$PATH` instead.
    #[cfg(feature = "scratch")]
    Tool {
        name: "scratch",
        about: "list the named sessions, or go into one",
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

/// Run a tool. The status is the process's.
///
/// Every tool answers `--help` and nothing else yet — the sub-subcommands are still to be written.
/// A stub that *says* it is a stub beats one that accepts arguments and ignores them: this way a
/// script built against a tool that does not do its job yet fails now rather than silently.
pub fn run(tool: &'static Tool, args: &[String]) -> i32 {
    if tool.name == "history" {
        return crate::cli::history::run(args);
    }
    // The same code the builtin runs, so the two answers cannot disagree about what is running.
    // `args` here has no command name in front of it, which the builtin's does.
    #[cfg(feature = "scratch")]
    if tool.name == "scratch" {
        // The overview page is this crate's, like `history`'s, so the two read alike. Everything
        // else is the shell's, and is the same code the builtin runs.
        if args.iter().any(|a| a == "--help" || a == "-h") {
            print!(
                "{}",
                crate::cli::scratch::text(crate::cli::help::Paint::detect())
            );
            return 0;
        }
        return oslo::env::builtins::scratch_tool(args);
    }
    let paint = crate::cli::help::Paint::detect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", help(tool, paint));
        return 0;
    }
    if let Some(unknown) = args.first() {
        eprint!("{}", help(tool, paint));
        eprintln!("\noslo {}: {unknown:?}: no such subcommand", tool.name);
        return 2;
    }
    print!("{}", help(tool, paint));
    0
}

/// A tool's own help.
fn help(tool: &'static Tool, paint: crate::cli::help::Paint) -> String {
    format!(
        "{}\n  {} {} {}\n\n{}\n  {}\n",
        paint.head("USAGE"),
        paint.key("oslo"),
        paint.key(tool.name),
        paint.slot("<subcommand> [...]"),
        paint.head("SUBCOMMANDS"),
        paint.dim("none yet — this tool is not implemented"),
    )
}

#[cfg(test)]
#[path = "tools/tests.rs"]
mod tests;
