//! `path` — the things you do to a filename, as a verb in the pipeline.
//!
//! ```sh
//! ls | path extension | distinct
//! path filter -x $PATH/*
//! if path is -d "$candidate"; then cd "$candidate"; fi
//! ```
//!
//! # Why it is here and not in `test`
//!
//! **`path filter -x` replaces a loop with a `test` in it.** That loop is the shape every script
//! ends up writing — `for f in …; do [ -x "$f" ] && printf '%s\n' "$f"; done` — and it is three
//! lines of quoting to say one thing. As a verb the same sentence is one stage, and what comes out
//! is rows, so the next stage does not have to take the lines apart again.
//!
//! `test` keeps every one of its operators: this is for *lists*, where `test` cannot reach, exactly
//! as `text` is for lists where parameter expansion cannot reach.
//!
//! # Where the paths come from
//!
//! The operands, then the rows, then the input lines — see [`super::scalar::gather`]. A row is read
//! from `path` first and then `name`, so `ls | path extension` works without naming a column.
//!
//! # What comes out
//!
//! Rows of one column, named `path`, like `text`. `is` is the exception and answers *no* rows: it
//! is a question, and its answer is the status — which is what lets it stand in an `if` where a
//! table would be noise.

use super::super::value::{Record, Val};
use super::scalar::{Wrong, refuse};
use std::path::Path;

/// The column every subcommand answers in.
const COLUMN: &str = "path";

/// Where a row's path is looked for. `name` is what `ls` calls it, which is the row this verb is
/// most often handed.
const READS: [&str; 3] = [COLUMN, "name", "line"];

/// Which subcommands take their own operand before the paths begin.
fn leading_operands(sub: &str) -> usize {
    match sub {
        "change-extension" => 1,
        _ => 0,
    }
}

/// Run `path`, or say why not.
pub fn run(
    words: &[String],
    input: Option<&[Record]>,
    bytes: Option<&str>,
) -> Option<(i32, Option<Vec<Record>>)> {
    let Some(sub) = words.get(1) else {
        let wrong = Wrong::at(
            "path",
            "no subcommand given",
            "name a subcommand, as in path basename",
        );
        return Some((refuse(words, "path", &wrong, &usage()), None));
    };

    let (flags, operands) = match split_flags(&words[2..]) {
        Ok(parsed) => parsed,
        Err(wrong) => {
            let verb = format!("path {sub}");
            return Some((refuse(words, &verb, &wrong, &usage()), None));
        }
    };

    let leading = leading_operands(sub);
    if operands.len() < leading {
        let wrong = Wrong::at(
            sub,
            "not enough operands",
            format!("needs {leading} more operand(s)"),
        );
        let verb = format!("path {sub}");
        return Some((refuse(words, &verb, &wrong, &usage()), None));
    }
    let own = &operands[..leading];
    let paths = gather(&operands[leading..], input, bytes);

    let outcome = match sub.as_str() {
        "basename" => Ok(each(&paths, |p| basename(p).to_string())),
        "dirname" => Ok(each(&paths, dirname)),
        "extension" => Ok(each(&paths, |p| extension(p).to_string())),
        "change-extension" => Ok(each(&paths, |p| change_extension(p, &own[0]))),
        "normalize" => Ok(each(&paths, normalize)),
        "resolve" => Ok(each(&paths, resolve)),
        "sort" => Ok(sort(&paths, &flags)),
        "mtime" => mtime(&paths, &flags),
        "filter" => Ok(filter(&paths, &flags)),
        // The one that answers a status instead of a table. See the module note.
        "is" => return Some((i32::from(!kept(&paths, &flags)), Some(Vec::new()))),
        other => {
            let wrong = Wrong::at(
                other,
                "no subcommand of that name",
                format!("{other}: not a subcommand"),
            );
            return Some((refuse(words, "path", &wrong, &usage()), None));
        }
    };

    match outcome {
        Ok(rows) => Some((0, Some(rows))),
        Err(wrong) => {
            let verb = format!("path {sub}");
            Some((refuse(words, &verb, &wrong, &usage()), None))
        }
    }
}

fn row(value: Val) -> Record {
    Record::from_pairs([(COLUMN, value)])
}

fn gather(operands: &[String], input: Option<&[Record]>, bytes: Option<&str>) -> Vec<String> {
    super::scalar::gather(operands, input, bytes, &READS)
}

/// Only the tests read a row back; `gather` does it for everything else.
#[cfg(test)]
fn string_of(record: &Record) -> String {
    super::scalar::string_of(record, &READS)
}

fn each(paths: &[String], how: impl Fn(&str) -> String) -> Vec<Record> {
    paths.iter().map(|p| row(Val::Str(how(p)))).collect()
}

/// The last component, with any trailing slashes off first.
///
/// `path basename /a/b/` is `b` rather than the empty string a naive split gives, because a
/// trailing slash says "this is a directory" and not "this ends in nothing".
fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return if path.is_empty() { "" } else { "/" };
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed)
}

/// Everything before the last component, and `.` when there is nothing before it.
fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(at) => trimmed[..at].to_string(),
        None => ".".to_string(),
    }
}

/// The extension, **without the dot** — `path extension a.rs` is `rs`.
///
/// Without it because that is the form everything else wants: a comparison, a `distinct`, the
/// operand to `change-extension`. A leading dot is a fact about where the extension sits in the
/// name, not part of the extension.
///
/// A dotfile has none: `.bashrc` is a name that begins with a dot, and reading `bashrc` as its
/// extension is the classic off-by-one this avoids.
fn extension(path: &str) -> &str {
    let name = basename(path);
    match name.rfind('.') {
        Some(0) | None => "",
        Some(at) => &name[at + 1..],
    }
}

/// `path change-extension EXT` — swap the extension, adding one where there was none.
///
/// The dot is optional in `EXT`, because both spellings are what someone means; an empty `EXT`
/// takes the extension off.
fn change_extension(path: &str, extension_to: &str) -> String {
    let wanted = extension_to.trim_start_matches('.');
    let name = basename(path);
    let stem = match name.rfind('.') {
        Some(0) | None => name,
        Some(at) => &name[..at],
    };
    let directory = &path[..path.len() - name.len()];
    match wanted.is_empty() {
        true => format!("{directory}{stem}"),
        false => format!("{directory}{stem}.{wanted}"),
    }
}

/// `.` and `..` resolved **without touching the disk**.
///
/// Lexical on purpose: it is the answer for a path that does not exist yet, which is most of the
/// paths a script builds. `resolve` is the one that asks the filesystem.
fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => match parts.last() {
                // At the root `..` is the root, which is what the kernel does too.
                Some(&"..") | None if !absolute => parts.push(".."),
                None => {}
                _ => {
                    parts.pop();
                }
            },
            other => parts.push(other),
        }
    }
    let joined = parts.join("/");
    match (absolute, joined.is_empty()) {
        (true, _) => format!("/{joined}"),
        (false, true) => ".".to_string(),
        (false, false) => joined,
    }
}

/// The real path, symlinks and all — or the normalized one when the filesystem cannot say.
///
/// **Falling back rather than failing**, because a path that does not exist still has an answer
/// worth having, and a verb that dropped those would be unusable on anything being built.
fn resolve(path: &str) -> String {
    if let Ok(real) = std::fs::canonicalize(path) {
        return real.to_string_lossy().into_owned();
    }
    if path.starts_with('/') {
        return normalize(path);
    }
    let here = std::env::current_dir().unwrap_or_default();
    normalize(&format!("{}/{path}", here.to_string_lossy()))
}

/// `path sort [--key basename|dirname] [--reverse]`.
fn sort(paths: &[String], flags: &Flags) -> Vec<Record> {
    let mut sorted: Vec<String> = paths.to_vec();
    match flags.key.as_deref() {
        Some("basename") => sorted.sort_by(|a, b| basename(a).cmp(basename(b))),
        Some("dirname") => sorted.sort_by_key(|p| dirname(p)),
        _ => sorted.sort(),
    }
    if flags.reverse {
        sorted.reverse();
    }
    each(&sorted, str::to_string)
}

/// `path mtime [--relative]` — when it was last written, in seconds.
///
/// Seconds since the epoch, or since *now* with `--relative`, which is the form a comparison wants:
/// `path mtime f | where 'path < 3600'` is "changed within the hour" without any date arithmetic.
fn mtime(paths: &[String], flags: &Flags) -> Result<Vec<Record>, Wrong> {
    let now = std::time::SystemTime::now();
    let mut rows = Vec::new();
    for path in paths {
        let written = std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .map_err(|problem| Wrong::plain(format!("{path}: {problem}")))?;
        let seconds = match flags.relative {
            true => now
                .duration_since(written)
                .map(|since| since.as_secs() as i64)
                .unwrap_or(0),
            false => written
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_secs() as i64)
                .unwrap_or(0),
        };
        rows.push(row(Val::Int(seconds)));
    }
    Ok(rows)
}

/// `path filter [-f|-d|-x|-r|-w|-l|-e] [--invert]` — the paths that pass every test given.
fn filter(paths: &[String], flags: &Flags) -> Vec<Record> {
    let passing: Vec<String> = paths
        .iter()
        .filter(|path| passes(path, flags) != flags.invert)
        .cloned()
        .collect();
    each(&passing, str::to_string)
}

/// Whether **every** path passes — what `path is` answers with a status.
///
/// Every rather than any, so `path is -d "$x"` on one path is `test -d "$x"` and on several is the
/// loop of them. An empty list passes nothing: `path is -f` with no paths is false, because there
/// is no file it found.
fn kept(paths: &[String], flags: &Flags) -> bool {
    !paths.is_empty() && paths.iter().all(|path| passes(path, flags) != flags.invert)
}

/// Every test the flags asked for, and existence when they asked for none.
fn passes(path: &str, flags: &Flags) -> bool {
    use nix::unistd::{AccessFlags, access};
    let as_path = Path::new(path);
    let metadata = std::fs::metadata(as_path);
    let mut asked = false;
    let mut ok = true;
    let mut test = |wanted: bool, answer: bool| {
        if wanted {
            asked = true;
            ok = ok && answer;
        }
    };
    test(flags.file, metadata.as_ref().is_ok_and(|m| m.is_file()));
    test(flags.dir, metadata.as_ref().is_ok_and(|m| m.is_dir()));
    test(
        flags.link,
        std::fs::symlink_metadata(as_path).is_ok_and(|m| m.file_type().is_symlink()),
    );
    // `access(2)` rather than the mode bits, for the reason `test -x` uses it: the mode says who
    // may, and the question is whether *this* process may.
    test(flags.executable, access(as_path, AccessFlags::X_OK).is_ok());
    test(flags.readable, access(as_path, AccessFlags::R_OK).is_ok());
    test(flags.writable, access(as_path, AccessFlags::W_OK).is_ok());
    test(flags.exists, metadata.is_ok());
    match asked {
        true => ok,
        false => as_path.exists(),
    }
}

/// Options, separated from operands. `--` ends them.
fn split_flags(words: &[String]) -> Result<(Flags, Vec<String>), Wrong> {
    let mut flags = Flags::default();
    let mut operands = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let word = &words[at];
        match word.as_str() {
            "--" => {
                operands.extend(words[at + 1..].iter().cloned());
                break;
            }
            "--file" | "-f" => flags.file = true,
            "--dir" | "-d" => flags.dir = true,
            "--link" | "-l" => flags.link = true,
            "--executable" | "-x" => flags.executable = true,
            "--readable" | "-r" => flags.readable = true,
            "--writable" | "-w" => flags.writable = true,
            "--exists" | "-e" => flags.exists = true,
            "--invert" | "-v" => flags.invert = true,
            "--reverse" => flags.reverse = true,
            "--relative" => flags.relative = true,
            "--key" => {
                let Some(value) = words.get(at + 1) else {
                    return Err(Wrong::at(
                        "--key",
                        "no key after it",
                        "--key: needs basename or dirname",
                    ));
                };
                if value != "basename" && value != "dirname" {
                    return Err(Wrong::at(
                        value,
                        "basename or dirname",
                        format!("--key: {value}: not basename or dirname"),
                    ));
                }
                flags.key = Some(value.clone());
                at += 1;
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(Wrong::at(
                    other,
                    "no option of that name",
                    format!("{other}: not an option"),
                ));
            }
            other => operands.push(other.to_string()),
        }
        at += 1;
    }
    Ok((flags, operands))
}

#[derive(Default)]
struct Flags {
    file: bool,
    dir: bool,
    link: bool,
    executable: bool,
    readable: bool,
    writable: bool,
    exists: bool,
    invert: bool,
    reverse: bool,
    relative: bool,
    key: Option<String>,
}

fn usage() -> String {
    "usage: path SUBCOMMAND [options] [paths…]\n\
     \n\
     \x20 basename | dirname | extension   the parts of a name\n\
     \x20 change-extension EXT             swap it; an empty EXT takes it off\n\
     \x20 normalize                        . and .. resolved without touching the disk\n\
     \x20 resolve                          the real path, symlinks and all\n\
     \x20 filter [-f -d -x -r -w -l -e]    the paths that pass every test given\n\
     \x20 is     [-f -d -x -r -w -l -e]    no rows: the status says whether they all did\n\
     \x20 sort [--key basename|dirname] [--reverse]\n\
     \x20 mtime [--relative]               seconds since the epoch, or since now\n\
     \n\
     The paths are the operands, or the rows, or the input lines — the first of those there is."
        .to_string()
}

#[cfg(test)]
mod tests;
