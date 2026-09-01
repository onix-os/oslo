//! Completions worked out from a command's own man page.
//!
//! ```text
//!   man rsync
//!        │
//!        ├─ rendered   `man -P cat`, with the overstrike bold taken back off
//!        ├─ sections   the headings whose name mentions OPTIONS
//!        └─ entries    a line that starts with `-`, and the prose under it
//! ```
//!
//! # Why this exists beside spec files
//!
//! A carapace spec is better than anything read from prose: it knows subcommands, argument
//! positions and what each of them completes to. It is also **absent for most of what is on a real
//! `$PATH`** — the local tools, the vendored scripts, the one binary this machine has that nobody
//! wrote a spec for. That long tail is what this is for, and it is why the bar is different: a spec
//! file is a promise, and this is a guess that has to be right often enough to be worth offering.
//!
//! # Honest about what it is
//!
//! **Man page formatting is not a format, it is a habit.** There is no grammar to conform to, and
//! the same page can spell a flag four ways. So the parse is deliberately narrow: it reads flags
//! and their descriptions and nothing else — no subcommands, no argument positions, no value
//! lists — and everything it is unsure of, it drops. **The failure mode has to be "no completion"
//! rather than a wrong one**, because a wrong flag offered with confidence is worse than a Tab that
//! does nothing.
//!
//! # Where it sits in the order
//!
//! Last. [`super::find`] looks in every spec directory first and only comes here when none of them
//! answered, so **a written spec always wins** and this can never take a command away from one.
//!
//! # When it runs
//!
//! On the first Tab that mentions a command, once per session: `oslo_ui::spec::custom` asks a
//! loader once per name and remembers the answer, including the answer "there is none". A machine
//! whose `$PATH` is all specs never runs `man` at all, and one whose commands have man pages pays
//! for each of them once.
//!
//! `OSLO_MAN_COMPLETION=0` turns it off.

use oslo_ui::spec::{Arg, CommandSpec, OptionSpec};
use std::collections::BTreeMap;

/// The spec a command's man page implies, if it has one worth having.
pub fn spec(command: &str) -> Option<CommandSpec> {
    if !enabled() || !is_a_command_name(command) {
        return None;
    }
    let page = render(command)?;
    from_page(command, &page)
}

/// Whether the man page source is turned on at all.
///
/// Off is `0`, `no` or `false` — a variable set to something else is somebody turning it *on* in
/// the way people usually mean, and reading that as off would be a surprise.
fn enabled() -> bool {
    match std::env::var("OSLO_MAN_COMPLETION") {
        Ok(value) => !matches!(value.trim(), "0" | "no" | "false" | "off"),
        Err(_) => true,
    }
}

/// A name that could be a command, and could not be anything else.
///
/// The same rule the spec directories use, and for the same reason: a word with a path in it is a
/// path, and handing one to `man` would be running `man` on whatever a completion had half typed.
fn is_a_command_name(command: &str) -> bool {
    !command.is_empty()
        && !command.contains('/')
        && !command.starts_with('-')
        && command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
}

/// The page, as plain text.
///
/// `-P cat` and the two `PAGER` variables, because a pager waiting for a keystroke is a shell that
/// has stopped. `LC_ALL=C` so the headings are the English ones this reads, and `GROFF_NO_SGR` so
/// the emphasis arrives as overstrike rather than as escape sequences — one of the two has to be
/// undone and overstrike is the one every `man` still produces.
fn render(command: &str) -> Option<String> {
    let output = std::process::Command::new("man")
        .args(["-P", "cat", command])
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", "80")
        .env("LC_ALL", "C")
        .env("GROFF_NO_SGR", "1")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(plain(&text))
}

/// Take the typography back off.
///
/// `man` renders bold as `X\bX` and italic as `_\bX`, which is how a line printer did it in 1978
/// and how every page still arrives. Left in, every flag would be spelled `--aallll`.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if chars.peek() == Some(&'\u{8}') {
            chars.next();
            // The character after the backspace is the one that survives: `_\bX` is an italic X,
            // and `X\bX` is a bold X.
            continue;
        }
        out.push(c);
    }
    out
}

/// The flags a rendered page declares.
fn from_page(command: &str, page: &str) -> Option<CommandSpec> {
    let mut found: BTreeMap<String, OptionSpec> = BTreeMap::new();
    for line in option_sections(page) {
        let Some(entry) = entry(&line.text) else {
            continue;
        };
        let described = match entry.description.is_empty() {
            true => line.following.clone(),
            false => entry.description,
        };
        // Keyed on the first spelling, so a page that lists `--all` twice — once in a summary and
        // once in the body — contributes one flag rather than two identical ones.
        let key = entry.names[0].clone();
        let option = OptionSpec {
            names: entry.names,
            description: shorten(&described),
            takes: match entry.takes_a_value {
                true => Arg::Required,
                false => Arg::None,
            },
            ..OptionSpec::default()
        };
        match found.get_mut(&key) {
            // **The described mention wins, wherever it came in the page.** `--help` is listed
            // bare in one section and explained in another on half the pages there are, and
            // keeping whichever came first left the explanation on the floor.
            Some(had) if had.description.is_empty() => *had = option,
            Some(_) => {}
            None => {
                found.insert(key, option);
            }
        }
    }
    // **Two is the floor.** A page that yielded one flag yielded it by accident far more often than
    // not, and a dropdown with one wrong entry in it is worse than no dropdown.
    if found.len() < 2 {
        return None;
    }
    Some(CommandSpec {
        name: command.to_string(),
        options: found.into_values().collect(),
        ..CommandSpec::default()
    })
}

/// One line of an options section, with the prose indented under it.
struct Line {
    text: String,
    following: String,
}

/// Every line of every section whose heading mentions options.
///
/// A heading in a rendered page is a line in **column zero**; everything else is indented. That is
/// the one piece of structure `man` output reliably has, and the whole reason this can find the
/// options at all rather than reading the page.
fn option_sections(page: &str) -> Vec<Line> {
    let lines: Vec<&str> = page.lines().collect();
    let mut out = Vec::new();
    let mut inside = false;
    for (at, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim_end();
        let is_heading = !trimmed.is_empty() && !trimmed.starts_with(char::is_whitespace);
        if is_heading {
            // `OPTIONS`, `COMMAND OPTIONS`, `GENERAL OPTIONS` — the habit is consistent about the
            // word even where it is inconsistent about everything else.
            inside = trimmed.to_ascii_uppercase().contains("OPTION");
            continue;
        }
        if !inside || trimmed.trim().is_empty() {
            continue;
        }
        out.push(Line {
            text: trimmed.to_string(),
            following: prose_under(&lines, at),
        });
    }
    out
}

/// The indented prose under a flag, when the flag's own line carried none.
///
/// Stops at the blank line, because that is where one entry ends and the next begins.
fn prose_under(lines: &[&str], at: usize) -> String {
    let indent = indent_of(lines[at]);
    let mut prose = String::new();
    for line in lines.iter().skip(at + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        // Back out to the flag's own column, or further: this is the next entry, not this one's
        // description.
        if indent_of(line) <= indent {
            break;
        }
        // **A word `man` broke across the margin is put back together.** Otherwise a description
        // reads `de- fault`, which is the page's line width showing through. Only a single hyphen
        // after a letter: `print SEP instead of --` ends a line too, and welding *that* to the next
        // word would invent a flag.
        let hyphenated = prose.ends_with('-')
            && !prose.ends_with("--")
            && prose
                .chars()
                .rev()
                .nth(1)
                .is_some_and(|c| c.is_ascii_alphabetic());
        match hyphenated {
            true => {
                prose.pop();
            }
            false if !prose.is_empty() => prose.push(' '),
            false => {}
        }
        prose.push_str(trimmed);
        // One sentence is a description; a paragraph is a manual.
        if prose.contains(". ") || prose.ends_with('.') {
            break;
        }
    }
    prose
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// The flags on one line, and whatever description shared it.
struct Entry {
    names: Vec<String>,
    takes_a_value: bool,
    description: String,
}

/// Read a line like `-a, --all              do not ignore entries starting with .`
///
/// **Where the flags stop is decided twice, and the stricter answer wins.** A run of two spaces is
/// a description for certain — `man` sets one that far from the flags it belongs to — but not every
/// page obliges: `grep` writes `--help Output a usage message and exit.` with a single space, and a
/// rule that only knew about the gap read the whole sentence as signature and lost the description.
///
/// So the words are also read one at a time, and the signature ends at the first that is neither a
/// flag nor a placeholder. `Output` is English; `FILE` is not.
fn entry(line: &str) -> Option<Entry> {
    let trimmed = line.trim();
    if !trimmed.starts_with('-') || trimmed == "-" || trimmed.starts_with("- ") {
        return None;
    }
    let (head, tail) = match trimmed.find("  ") {
        Some(at) => (&trimmed[..at], trimmed[at..].trim()),
        None => (trimmed, ""),
    };

    let mut names = Vec::new();
    let mut takes_a_value = false;
    let mut prose = String::new();
    let mut at = 0;
    while at < head.len() {
        let rest = &head[at..];
        let skipped = rest.len() - rest.trim_start().len();
        at += skipped;
        if at >= head.len() {
            break;
        }
        let word = head[at..].split_whitespace().next().unwrap_or_default();
        match read_word(word) {
            Some(read) => {
                names.extend(read.names);
                takes_a_value = takes_a_value || read.takes_a_value;
                at += word.len();
            }
            // English. Everything from here on is what the flag does.
            None => {
                prose = head[at..].trim().to_string();
                break;
            }
        }
    }
    if names.is_empty() {
        return None;
    }
    let description = match (prose.is_empty(), tail.is_empty()) {
        (true, _) => tail.to_string(),
        (false, true) => prose,
        (false, false) => format!("{prose} {tail}"),
    };
    Some(Entry {
        names,
        takes_a_value,
        description,
    })
}

/// What one word of a signature contributes, or `None` if it is not part of one.
///
/// A word can hold more than one flag — `-a,--all` with no space — and it can hold a flag and its
/// argument: `--width=COLS`, `--color[=WHEN]`.
fn read_word(word: &str) -> Option<Signature> {
    let mut names = Vec::new();
    let mut takes_a_value = false;
    // `,`, `|` and `/` all appear as "or" between two spellings of one flag; a bracket is how an
    // optional part is written around one.
    for piece in word.split([',', '|', '/', '[', ']', '(', ')']) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        if !piece.starts_with('-') {
            // `-o FILE` puts the argument in its own word; `--color[=WHEN]` leaves `=WHEN` here.
            if !is_a_placeholder(piece.trim_start_matches('=')) {
                return None;
            }
            takes_a_value = true;
            continue;
        }
        let (name, after) = match piece.split_once('=') {
            Some((name, after)) => (name, after),
            None => (piece, ""),
        };
        if !is_a_flag(name) {
            return None;
        }
        // `--file=` with nothing after it still says the flag takes one.
        takes_a_value = takes_a_value || piece.contains('=') || is_a_placeholder(after);
        names.push(name.to_string());
    }
    Some(Signature {
        names,
        takes_a_value,
    })
}

struct Signature {
    names: Vec<String>,
    takes_a_value: bool,
}

/// `-a`, `--all`, `-W`. Not `--`, not `-`, and not a word that merely begins with a dash.
fn is_a_flag(word: &str) -> bool {
    let name = word.trim_start_matches('-');
    let dashes = word.len() - name.len();
    (1..=2).contains(&dashes)
        && !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// `FILE`, `<path>`, `NUM` — a stand-in for a value rather than a word of English.
///
/// Upper case or angle brackets, and nothing else. Every looser rule reads the first word of a
/// description as an argument, which turns a switch into a flag that swallows the next word.
fn is_a_placeholder(word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    if word.starts_with('<') && word.ends_with('>') {
        return true;
    }
    word.chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.'))
        && word.chars().any(|c| c.is_ascii_uppercase())
}

/// One line of description, at most.
///
/// The dropdown has one row per flag and the prose in a man page has no length limit, so something
/// has to decide; the first sentence is what the writer put first.
fn shorten(text: &str) -> String {
    // **Justified text, un-justified.** `man` pads between words to reach the margin, so a
    // description lifted straight out reads `this  can  be  helpful` — the page's line width
    // showing through in a dropdown that has nothing to do with it.
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = text.trim();
    let first = match text.find(". ") {
        Some(at) => &text[..at + 1],
        None => text,
    };
    // The full stop that ends a sentence, not one that *is* the description: `ls` explains `-a` as
    // "do not ignore entries starting with .", and stripping that leaves a sentence about nothing.
    let first = match first.ends_with('.') && !first.ends_with(" .") {
        true => first[..first.len() - 1].trim_end(),
        false => first.trim_end(),
    };
    match first.char_indices().nth(90) {
        Some((at, _)) => format!("{}…", first[..at].trim_end()),
        None => first.to_string(),
    }
}

#[cfg(test)]
#[path = "man/tests.rs"]
mod tests;
