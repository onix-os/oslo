//! `rm`, as a builtin, with the conveniences confined to where they cannot hurt a script.
//!
//! # Why a builtin `rm` is a careful thing to write
//!
//! A builtin shadows `/bin/rm` for **everything** the shell runs, and oslo is meant to be
//! `/bin/sh` on a distribution. Every `postinst`, every `configure`, every `Makefile` recipe on
//! the machine would reach this code. So the rule here is narrow and mechanical:
//!
//! **In a non-interactive shell, `rm` does exactly what `rm` has always done.** No trash, and a
//! directory operand without `-r` is an error. The extensions exist only at a prompt, where a
//! person typed the line and can see what happened. `-s`/`--strict` asks for the same strictness
//! at a prompt, for the times you mean it.
//!
//! That is the same gate `crate::expand::sugar` puts on `=command` and `@name`, for the same
//! reason: a convenience that changes what a script means is not a convenience.
//!
//! # And why an unknown option is not an error
//!
//! GNU's `rm` has options this does not implement — `--one-file-system`, `--preserve-root=all`,
//! `-I`. A script using one of them must not break because oslo took the name over, so an option
//! this does not recognise hands the whole invocation to the real `rm` on `$PATH`. The builtin can
//! therefore never be *less* capable than the system's, which is the only honest way to shadow a
//! command everything depends on.

mod trash;

use crate::env::options::ShellOption;
use crate::env::scope::Environment;
use crate::error::Result;
use std::path::{Path, PathBuf};

/// What the options add up to.
struct Options {
    /// `-f`: missing operands are not errors, and nothing is ever prompted for.
    force: bool,
    /// `-i`: prompt before each removal.
    interactive: bool,
    /// `-r`/`-R`: recurse into directories.
    recursive: bool,
    /// `-d`: remove an empty directory, as `rmdir` would.
    dir: bool,
    /// `-v`: say what was removed.
    verbose: bool,
    /// `-s`/`--strict`: POSIX behaviour, even at a prompt.
    strict: bool,
}

/// How this invocation should behave, once the options and the session are both known.
struct Mode {
    /// Whether a directory may be removed without `-r`.
    loose: bool,
    /// Where a removal should move things, or `None` to unlink them.
    trash: Option<trash::Trash>,
}

pub fn builtin_rm(env: &mut Environment, args: &[String]) -> Result<i32> {
    let (options, operands) = match parse(args) {
        Parsed::Options(options, operands) => (options, operands),
        // An option this does not implement: the real `rm` gets the whole line, unchanged.
        Parsed::Delegate => return delegate(args),
        Parsed::Usage(message) => {
            eprintln!("oslo: rm: {message}");
            return Ok(2);
        }
    };

    if operands.is_empty() {
        if options.force {
            // `rm -f` with nothing to remove is POSIX's one silent success.
            return Ok(0);
        }
        eprintln!("oslo: rm: missing operand");
        return Ok(1);
    }

    let mode = mode_for(env, &options);
    let mut status = 0;
    for operand in operands {
        if !remove_operand(Path::new(operand), operand, &options, &mode) {
            status = 1;
        }
    }
    Ok(status)
}

/// The behaviour this shell allows, which is the whole safety argument in five lines.
fn mode_for(env: &Environment, options: &Options) -> Mode {
    let at_a_prompt = env.options().is_set(ShellOption::Interactive);
    if options.strict || !at_a_prompt {
        return Mode {
            loose: false,
            trash: None,
        };
    }
    let settings = crate::ui::settings::current().builtin.rm;
    Mode {
        loose: true,
        trash: settings.to_tmp.then(|| trash::Trash::new(&settings)),
    }
}

/// Remove one operand. `true` if it is gone (or was never there under `-f`).
fn remove_operand(path: &Path, shown: &str, options: &Options, mode: &Mode) -> bool {
    // `symlink_metadata`, never `metadata`: `rm link-to-dir` removes the link and must not
    // recurse into what it points at. That distinction is the difference between deleting one
    // entry and deleting someone's home directory.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if options.force {
                return true;
            }
            eprintln!("oslo: rm: cannot remove '{shown}': No such file or directory");
            return false;
        }
        Err(e) => {
            eprintln!("oslo: rm: cannot remove '{shown}': {e}");
            return false;
        }
    };

    if let Some(refusal) = refuse(path, shown, &meta, options, mode) {
        eprintln!("oslo: rm: {refusal}");
        return false;
    }

    if options.interactive && !options.force && !confirm(shown, &meta) {
        return true;
    }

    let directory = meta.is_dir();
    if let Some(trash) = &mode.trash
        && let Some(outcome) = trash.take(path, shown, directory)
    {
        return match outcome {
            Ok(moved) => {
                if options.verbose {
                    println!("moved '{shown}' to '{}'", moved.display());
                }
                true
            }
            Err(e) => {
                // **Not a fallback to destroying it.** The whole point of the trash is that a
                // removal is recoverable; quietly unlinking a file the move could not save would
                // be the one failure the user is relying on this not to have.
                eprintln!("oslo: rm: cannot move '{shown}' to the trash: {e}");
                false
            }
        };
    }

    // **`-d` on its own is `rmdir`, not `rm -r`.** It removes an *empty* directory and fails on
    // one with anything in it, which is the whole difference between the two flags — reaching for
    // `remove_dir_all` here deleted a tree that GNU refuses to touch.
    let removed = match (directory, options.recursive || mode.loose) {
        (true, true) => std::fs::remove_dir_all(path),
        (true, false) => std::fs::remove_dir(path),
        (false, _) => std::fs::remove_file(path),
    };
    match removed {
        Ok(()) => {
            if options.verbose {
                println!("removed '{shown}'");
            }
            true
        }
        Err(e) => {
            eprintln!("oslo: rm: cannot remove '{shown}': {e}");
            false
        }
    }
}

/// Why this operand must not be removed, if it must not be.
fn refuse(
    path: &Path,
    shown: &str,
    meta: &std::fs::Metadata,
    options: &Options,
    mode: &Mode,
) -> Option<String> {
    // POSIX names these two explicitly, and the reason is not pedantry: `rm -r .` would walk
    // the working directory, and `rm -r ..` its parent, from a line that looks local.
    if ends_in_dot(shown) {
        return Some(format!(
            "refusing to remove '.' or '..' directory: skipping '{shown}'"
        ));
    }
    if !meta.is_dir() {
        return None;
    }
    // Only the real root, resolved — so a symlink pointing at `/` is caught too, and a directory
    // merely *named* `/` in a longer path is not.
    if path.canonicalize().is_ok_and(|full| full == Path::new("/")) {
        return Some("it is dangerous to operate recursively on '/'".to_string());
    }
    if options.recursive || options.dir || mode.loose {
        return None;
    }
    Some(format!("cannot remove '{shown}': Is a directory"))
}

/// Whether the last component of an operand is `.` or `..`.
///
/// **Done on the text, not on a `Path`.** `Path::file_name` answers `None` for anything ending in
/// `..`, and `Path::components` drops a trailing `.` entirely — so both of the things this exists
/// to catch are invisible to the path API. `a/..` reached `remove_dir_all` on the parent directory
/// until this was written against the string instead.
fn ends_in_dot(operand: &str) -> bool {
    let trimmed = operand.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    // The empty case is `/` and its friends, which `refuse` catches by canonicalising instead.
    matches!(last, "." | "..")
}

/// `-i`. Anything but a `y` answer means no, as it does in `rm` and in `find -ok`.
fn confirm(shown: &str, meta: &std::fs::Metadata) -> bool {
    let kind = if meta.is_dir() { "directory" } else { "file" };
    eprint!("oslo: rm: remove {kind} '{shown}'? ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut answer = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut answer) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(answer.trim_start().chars().next(), Some('y') | Some('Y')),
    }
}

/// What the argument list turned out to be.
enum Parsed<'a> {
    Options(Options, Vec<&'a String>),
    /// An option this does not implement. The real `rm` decides.
    Delegate,
    Usage(String),
}

fn parse(args: &[String]) -> Parsed<'_> {
    let mut options = Options {
        force: false,
        interactive: false,
        recursive: false,
        dir: false,
        verbose: false,
        strict: false,
    };
    let mut operands: Vec<&String> = Vec::new();
    let mut only_operands = false;

    for arg in args.iter().skip(1) {
        if only_operands || !arg.starts_with('-') || arg == "-" {
            operands.push(arg);
            continue;
        }
        if arg == "--" {
            only_operands = true;
            continue;
        }
        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "force" => options.force = true,
                "recursive" => options.recursive = true,
                "interactive" => options.interactive = true,
                "dir" => options.dir = true,
                "verbose" => options.verbose = true,
                "strict" => options.strict = true,
                "help" => return Parsed::Usage(HELP.to_string()),
                _ => return Parsed::Delegate,
            }
            continue;
        }
        for letter in arg.chars().skip(1) {
            match letter {
                // `-f` and `-i` are the same knob from opposite ends, and the *last* one wins —
                // which is why each clears the other rather than only setting itself.
                'f' => {
                    options.force = true;
                    options.interactive = false;
                }
                'i' => {
                    options.interactive = true;
                    options.force = false;
                }
                'r' | 'R' => options.recursive = true,
                'd' => options.dir = true,
                'v' => options.verbose = true,
                's' => options.strict = true,
                _ => return Parsed::Delegate,
            }
        }
    }
    Parsed::Options(options, operands)
}

const HELP: &str = "usage: rm [-dfirRvs] [--strict] file...";

/// Hand the invocation to the real `rm`.
fn delegate(args: &[String]) -> Result<i32> {
    let Some(program) = external_rm() else {
        eprintln!("oslo: rm: unknown option, and no rm on PATH to hand it to");
        return Ok(2);
    };
    super::spawn::run_external(&program, args, "rm")
}

/// The `rm` that is not this one.
///
/// `$PATH` first, so a machine that puts its coreutils somewhere unusual still works, then the two
/// places `rm` has lived for forty years — needed because a shell that has just been made
/// `/bin/sh` may be running with a `$PATH` that has not been set up yet.
fn external_rm() -> Option<PathBuf> {
    if let Some(found) = super::spawn::resolve_program("rm")
        && found != Path::new("/usr/bin/oslo")
    {
        return Some(found);
    }
    ["/usr/bin/rm", "/bin/rm"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

#[cfg(test)]
#[path = "remove/tests.rs"]
mod tests;
