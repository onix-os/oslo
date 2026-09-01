//! `oslo fmt` — lay a shell script out, using the parser the shell already has.
//!
//! ```sh
//! oslo fmt script.sh            # to standard output
//! oslo fmt -w script.sh *.sh    # in place
//! oslo fmt --check .            # status 1 and a list, changing nothing
//! cat script.sh | oslo fmt      # a filter
//! ```
//!
//! # Why oslo has one and no other shell does
//!
//! **The parser already paid for it.** rune's tree holds every byte of the input, so a formatter is
//! not a second implementation of shell that prints a program back out — it is a walk that changes
//! the space *between* tokens and copies everything else. A construct it has never heard of comes
//! out as the text it was. See `rune::format` for the two invariants and what is never touched.
//!
//! # The division
//!
//! The engine is rune's and the command is oslo's, the same split as parsing and lowering: rune
//! owns the tree and the guarantees about it, oslo owns the verb a person types. Anything else that
//! wants to format shell gets the engine without taking the shell with it.
//!
//! # What it refuses
//!
//! A script that does not parse. There is no tree worth reformatting under a missing `fi`, and the
//! output would be a second mistake laid over the first — so the file is left exactly as it is and
//! the errors are reported with their line numbers.

use std::io::Read;
use std::path::{Path, PathBuf};

/// What to do with the result.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Write it to standard output. The default, and what makes it usable in a pipe.
    Print,
    /// Write it back over the file.
    Write,
    /// Change nothing, and answer 1 if anything would have changed.
    Check,
}

/// `oslo fmt [options] [FILE...]`.
pub fn run(args: &[String]) -> i32 {
    let mut mode = Mode::Print;
    let mut indent = "    ".to_string();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut only_operands = false;

    let mut at = 0;
    while at < args.len() {
        let word = args[at].as_str();
        match word {
            _ if only_operands => paths.push(PathBuf::from(word)),
            "--" => only_operands = true,
            "-h" | "--help" => {
                print!("{}", help());
                return 0;
            }
            "-w" | "--write" => mode = Mode::Write,
            "--check" => mode = Mode::Check,
            "--tabs" => indent = "\t".to_string(),
            "--indent" => {
                let Some(value) = args.get(at + 1) else {
                    eprintln!("oslo fmt: --indent: needs a number of spaces");
                    return 2;
                };
                let Ok(width) = value.parse::<usize>() else {
                    eprintln!("oslo fmt: --indent: {value}: not a number");
                    return 2;
                };
                indent = " ".repeat(width);
                at += 1;
            }
            "-" => paths.push(PathBuf::from("-")),
            other if other.starts_with('-') => {
                eprintln!("oslo fmt: {other}: no option of that name");
                eprint!("{}", help());
                return 2;
            }
            other => paths.push(PathBuf::from(other)),
        }
        at += 1;
    }

    let options = rune::FormatOptions { indent };

    // No files is standard input, which is what makes `… | oslo fmt` a filter rather than an error.
    if paths.is_empty() {
        if mode != Mode::Print {
            eprintln!("oslo fmt: -w and --check need files to work on");
            return 2;
        }
        return from_stdin(&options);
    }

    let files = match gather(&paths) {
        Ok(files) => files,
        Err(status) => return status,
    };

    let mut worst = 0;
    let mut would_change = Vec::new();
    for file in &files {
        match one(file, mode, &options) {
            Outcome::Same => {}
            Outcome::Changed => would_change.push(file.clone()),
            Outcome::Failed(status) => worst = worst.max(status),
        }
    }

    if mode == Mode::Check && !would_change.is_empty() {
        for file in &would_change {
            println!("{}", file.display());
        }
        return worst.max(1);
    }
    worst
}

/// What happened to one file.
enum Outcome {
    Same,
    Changed,
    Failed(i32),
}

fn one(path: &Path, mode: Mode, options: &rune::FormatOptions) -> Outcome {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(problem) => {
            eprintln!("oslo fmt: {}: {problem}", path.display());
            return Outcome::Failed(2);
        }
    };
    let formatted = match rune::format_with(&text, options) {
        Ok(formatted) => formatted,
        Err(errors) => {
            complain(path, &text, &errors);
            return Outcome::Failed(2);
        }
    };

    if mode == Mode::Print {
        print!("{formatted}");
        return Outcome::Same;
    }
    if formatted == text {
        return Outcome::Same;
    }
    if mode == Mode::Check {
        return Outcome::Changed;
    }
    // **Written only when it differs.** Rewriting an already-formatted file would move its
    // modification time, and everything that watches a directory would rebuild for nothing.
    match std::fs::write(path, &formatted) {
        Ok(()) => Outcome::Changed,
        Err(problem) => {
            eprintln!("oslo fmt: {}: {problem}", path.display());
            Outcome::Failed(2)
        }
    }
}

fn from_stdin(options: &rune::FormatOptions) -> i32 {
    let mut text = String::new();
    if let Err(problem) = std::io::stdin().read_to_string(&mut text) {
        eprintln!("oslo fmt: standard input: {problem}");
        return 2;
    }
    match rune::format_with(&text, options) {
        Ok(formatted) => {
            print!("{formatted}");
            0
        }
        Err(errors) => {
            complain(Path::new("<stdin>"), &text, &errors);
            2
        }
    }
}

/// Say what could not be read, and where.
///
/// Every error rather than the first: a script with four mistakes in it is four things to fix, and
/// reporting one at a time is how a formatter becomes a thing people run four times.
fn complain(path: &Path, text: &str, errors: &[rune::Error]) {
    let source = rune::Source::new(text);
    eprintln!(
        "oslo fmt: {}: not formatted, because it does not parse",
        path.display()
    );
    for error in errors {
        let (line, column) = source.line_col(error.span.start);
        eprintln!("  {}:{line}:{column}: {}", path.display(), error.message);
    }
}

/// The files named, with a directory standing for the scripts under it.
///
/// A directory is expanded rather than refused because `oslo fmt --check .` is the shape every
/// pre-commit hook wants, and asking for a `find` in front of it is asking for the hook to be
/// written differently in every project.
fn gather(paths: &[PathBuf]) -> Result<Vec<PathBuf>, i32> {
    let mut files = Vec::new();
    for path in paths {
        if path.as_os_str() == "-" {
            eprintln!("oslo fmt: - names standard input, which cannot be mixed with files");
            return Err(2);
        }
        match path.is_dir() {
            true => under(path, &mut files),
            false => files.push(path.clone()),
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Every `.sh` and `.bash` under a directory.
fn under(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        eprintln!("oslo fmt: {}: cannot be read", directory.display());
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // A dotted directory is somebody else's — `.git` most of all, where a hook is a shell
        // script and rewriting one is not what `oslo fmt .` was asked to do.
        if path.file_name().is_some_and(|name| {
            name.to_str()
                .is_some_and(|name| name.starts_with('.') && path.is_dir())
        }) {
            continue;
        }
        match path.is_dir() {
            true => under(&path, found),
            false => {
                if path
                    .extension()
                    .is_some_and(|ext| ext == "sh" || ext == "bash")
                {
                    found.push(path);
                }
            }
        }
    }
}

fn help() -> String {
    let paint = crate::cli::help::Paint::detect();
    format!(
        "{}\n  {} {} {}\n\n{}\n  {}\n\n{}\n  {}   write the result back over each file\n  \
         {}       change nothing; status 1, and a list, if anything would\n  \
         {}    one level of indentation, in spaces (default 4)\n  \
         {}        a tab instead\n\n\
         With no files, reads standard input and writes to standard output. A directory stands for\n\
         the .sh and .bash files under it. A script that does not parse is left alone.\n",
        paint.head("USAGE"),
        paint.key("oslo"),
        paint.key("fmt"),
        paint.slot("[options] [FILE...]"),
        paint.head("WHAT IT DOES"),
        "lays out a shell script: indentation, spacing, and where the keywords sit",
        paint.head("OPTIONS"),
        paint.key("-w, --write"),
        paint.key("--check"),
        paint.key("--indent N"),
        paint.key("--tabs"),
    )
}

#[cfg(test)]
mod tests;
