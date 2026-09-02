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

// Lining up a script's argc declarations. Behind `argc`, so a build that cannot *run* one does not
// tidy one either.
#[cfg(feature = "argc")]
mod argc;

use std::io::Read;
use std::path::{Path, PathBuf};

/// Format one script: rune's walk, and then whatever oslo knows that rune does not.
///
/// **The order is the whole of it.** rune returns every comment byte for byte, so the argc pass
/// below is looking at the lines their author wrote — it never has to wonder whether something
/// earlier moved them.
fn lay_out(text: &str, options: &rune::FormatOptions) -> Result<String, Vec<rune::Error>> {
    let formatted = rune::format_with(text, options)?;
    #[cfg(feature = "argc")]
    let formatted = argc::align(&formatted);
    Ok(formatted)
}

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

/// Refuse an option, with a caret under the word that was wrong.
///
/// **The tool's own name goes back on the front.** `crate::env::complain` draws by finding `word`
/// among the command's words and pointing into the line they rebuild — and `args` here starts
/// *after* `oslo fmt`, so a line built from it alone would put the caret several columns left of
/// where it belongs. See `docs/features/diagnostics.md`.
/// **The origin is empty on purpose.** A builtin's complaint is prefixed with `oslo: ` — or with a
/// file and a line inside a script — because that is where it came from. This is not a builtin: it
/// is the `oslo` program, whose messages have always begun `oslo fmt: `, and letting the prefix be
/// added would have printed `oslo: oslo fmt: …`. The one rule is that a pipe sees the bytes it
/// always saw, and that includes the ones at the front.
fn refuse(args: &[String], word: &str, body: &str, label: &str) -> i32 {
    let mut words = vec!["oslo".to_string(), "fmt".to_string()];
    words.extend(args.iter().cloned());
    let usage = help();
    if !oslo::env::complain_from("", &words, word, body, label, Some(usage.trim_end())) {
        eprint!("{usage}");
    }
    2
}

/// `oslo fmt [options] [FILE...]`.
pub fn run(args: &[String]) -> i32 {
    let mut mode = Mode::Print;
    // The word that asked for the mode, so a complaint about it points at what was typed.
    let mut mode_word = String::new();
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
            "-w" | "--write" => {
                mode = Mode::Write;
                mode_word = word.to_string();
            }
            "--check" => {
                mode = Mode::Check;
                mode_word = word.to_string();
            }
            "--tabs" => indent = "\t".to_string(),
            "--indent" => {
                let Some(value) = args.get(at + 1) else {
                    return refuse(
                        args,
                        "--indent",
                        "oslo fmt: --indent: needs a number of spaces",
                        "no number after it",
                    );
                };
                let Ok(width) = value.parse::<usize>() else {
                    return refuse(
                        args,
                        value,
                        &format!("oslo fmt: --indent: {value}: not a number"),
                        "not a number",
                    );
                };
                indent = " ".repeat(width);
                at += 1;
            }
            "-" => paths.push(PathBuf::from("-")),
            other if other.starts_with('-') => {
                return refuse(
                    args,
                    other,
                    &format!("oslo fmt: {other}: no option of that name"),
                    "no option of that name",
                );
            }
            other => paths.push(PathBuf::from(other)),
        }
        at += 1;
    }

    let options = rune::FormatOptions { indent };

    // No files is standard input, which is what makes `… | oslo fmt` a filter rather than an error.
    if paths.is_empty() {
        if mode != Mode::Print {
            return refuse(
                args,
                &mode_word,
                "oslo fmt: -w and --check need files to work on",
                "this needs a file to work on",
            );
        }
        return from_stdin(&options);
    }

    // **Standard input is the default, so naming it beside a file is two sources for one output.**
    // Checked here rather than in `gather`, which has the paths but not the words the caret needs.
    if paths.iter().any(|path| path.as_os_str() == "-") {
        return refuse(
            args,
            "-",
            "oslo fmt: - names standard input, which cannot be mixed with files",
            "standard input, beside a file",
        );
    }

    let files = gather(&paths);

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
    let formatted = match lay_out(&text, options) {
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
    match lay_out(&text, options) {
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
fn gather(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        match path.is_dir() {
            true => under(path, &mut files),
            false => files.push(path.clone()),
        }
    }
    files.sort();
    files.dedup();
    files
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
