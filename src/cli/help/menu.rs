//! One renderer for every tool's help, so that all of them read as pages of one manual.
//!
//! # Why this exists
//!
//! `history`, `plugin` and `macros` each carried their own copy of the same ninety lines, and
//! `profile` and `secret` had hand-written `usage:` strings instead — different headings, different
//! colour, different wording for the same idea. A person who has learned one tool's help has
//! learned nothing about the next, which is the whole cost of writing it five times.
//!
//! So a tool declares [`Sub`] rows and a [`Menu`], and everything about how those are drawn — the
//! headings, the painter, the column, the wrapping, what an unknown subcommand prints — is decided
//! exactly once, here.
//!
//! # The shape every page has
//!
//! ```text
//! USAGE
//!   oslo secret key <subcommand> [argument]...
//!
//! SUBCOMMANDS
//!   add                 …
//!
//!   `oslo secret key <subcommand> --help` for that subcommand's arguments.
//! ```
//!
//! and one subcommand's page:
//!
//! ```text
//! USAGE
//!   oslo secret key add file PATH
//!
//!   read the identity out of a file
//!
//! ARGUMENTS
//!   --store NAME        …
//!
//!   The note, wrapped to the terminal.
//! ```

use super::{Paint, row};
use std::fmt::Write as _;

/// One row of a menu: a subcommand, or an operand where a tool takes those instead.
pub struct Sub {
    pub name: &'static str,
    /// Argument syntax, as it is written after the name.
    pub args: &'static str,
    /// The one line that goes beside the name in the list.
    pub about: &'static str,
    /// What `--help` on this one lists under ARGUMENTS.
    pub flags: &'static [(&'static str, &'static str)],
    /// The paragraph under that, wrapped to the terminal. Empty for a row that needs none.
    pub note: &'static str,
}

/// The usual heading for the list.
pub const SUBCOMMANDS: &str = "SUBCOMMANDS";

/// For a tool whose arguments are named things rather than reserved words.
///
/// `scratch` is the only one, and it is behind a cargo feature — so a build without it has a
/// heading nobody uses rather than a heading that is wrong.
#[allow(dead_code)]
pub const OPERANDS: &str = "OPERANDS";

/// What nearly every tool's USAGE line says after its name.
pub const CALL: &str = "<subcommand> [argument]...";

/// What to call a word a subcommand cannot use.
///
/// A leading `-` is somebody reaching for a flag that is not there, and "too many arguments" would
/// send them counting operands instead of checking the spelling.
pub fn describe_extra(word: &str) -> String {
    if word.starts_with('-') && word.len() > 1 {
        format!("{word:?}: unknown option")
    } else {
        format!("{word:?}: too many arguments")
    }
}

/// A tool's help page, and the pages of the things under it.
pub struct Menu {
    /// The words before the subcommand — `["secret"]`, or `["secret", "key"]` for a nested one.
    pub path: &'static [&'static str],
    /// What the USAGE line says after the path: [`CALL`] unless the tool takes operands.
    pub call: &'static str,
    /// What the list is called: [`SUBCOMMANDS`] for nearly everything.
    pub heading: &'static str,
    pub subs: &'static [Sub],
    /// Dim lines under the list, for what is true of the whole tool rather than of one row.
    pub notes: &'static [&'static str],
    /// Menus of their own, for rows that are a group of commands rather than a command.
    ///
    /// `oslo secret key --help` then answers with what `key` can be asked, rather than with the one
    /// line the list above already showed.
    pub nested: &'static [&'static Menu],
}

impl Menu {
    /// The overview: the list, and where to go for one row's arguments.
    pub fn overview(&self, paint: Paint) -> String {
        let mut text = String::new();
        let _ = writeln!(text, "{}", paint.head("USAGE"));
        let _ = writeln!(
            text,
            "  {}{} {}",
            paint.key("oslo"),
            self.painted_path(paint),
            paint.slot(self.call)
        );
        let _ = writeln!(text, "\n{}", paint.head(self.heading));
        for sub in self.subs {
            text.push_str(&row(sub.name, paint.key(sub.name), sub.about));
        }
        // Said only where it leads somewhere: a menu whose rows have neither arguments nor a note
        // would be pointing at a page that turns out to be the line you already read.
        if self.subs.iter().any(|sub| sub.has_a_page()) {
            let _ = writeln!(
                text,
                "\n  {}",
                paint.dim(&format!(
                    "`oslo{} <subcommand> --help` for that subcommand's arguments.",
                    self.plain_path()
                ))
            );
        } else {
            text.push('\n');
        }
        for note in self.notes {
            let _ = writeln!(text, "  {}", paint.dim(note));
        }
        text
    }

    /// The page these arguments are asking for, or `None` when they are asking for work instead.
    ///
    /// **Both spellings, in every tool.** `oslo history list --help` and `oslo history --help list`
    /// are the same question, and a person who learned one of them in one tool has learned it in
    /// all of them. A name nobody listed falls back to the overview rather than to nothing, because
    /// the overview is what somebody who got the name wrong needs to read.
    ///
    /// An empty `args` is deliberately not handled here: a bare `oslo history` is a help page and a
    /// bare `oslo secret` is a listing, and that is each tool's decision rather than this one's.
    pub fn asked(&self, args: &[String], paint: Paint) -> Option<String> {
        let word = |at: usize| args.get(at).map(String::as_str);
        let is_help = |word: Option<&str>| matches!(word, Some("-h" | "--help" | "help"));

        let name = if is_help(word(0)) {
            word(1)
        } else if is_help(word(1)) {
            word(0)
        } else {
            return None;
        };
        Some(match name.and_then(|name| self.subcommand(name, paint)) {
            Some(page) => page,
            None => self.overview(paint),
        })
    }

    /// One row's own page, or `None` when nothing by that name is listed.
    ///
    /// A row that leads to a menu of its own answers with that menu, because what somebody asking
    /// about `key` wants is what `key` can be asked.
    pub fn subcommand(&self, name: &str, paint: Paint) -> Option<String> {
        if let Some(nested) = self.nested.iter().find(|menu| menu.leaf() == name) {
            return Some(nested.overview(paint));
        }
        let sub = self.subs.iter().find(|sub| sub.name == name)?;
        let mut text = String::new();
        let _ = writeln!(text, "{}", paint.head("USAGE"));
        let _ = write!(
            text,
            "  {}{} {}",
            paint.key("oslo"),
            self.painted_path(paint),
            paint.key(sub.name)
        );
        if !sub.args.is_empty() {
            let _ = write!(text, " {}", paint.slot(sub.args));
        }
        let _ = writeln!(text, "\n\n  {}", sub.about);

        if !sub.flags.is_empty() {
            let _ = writeln!(text, "\n{}", paint.head("ARGUMENTS"));
            for (flag, about) in sub.flags {
                text.push_str(&row(flag, paint.key(flag), about));
            }
        }
        if !sub.note.is_empty() {
            text.push('\n');
            for line in wrapped(sub.note) {
                let _ = writeln!(text, "  {}", paint.dim(&line));
            }
        }
        Some(text)
    }

    /// What a word nobody listed gets: the page, then what was wrong with it.
    ///
    /// **The complaint goes last**, because it is the line that scrolls away otherwise — and the
    /// page above it is the answer to the question the person was about to ask. Both on standard
    /// error, so a `--help` piped into something is not polluted by a failure that went to the same
    /// place.
    pub fn unknown(&self, word: &str) -> i32 {
        eprint!("{}", self.overview(Paint::plain()));
        eprintln!("\noslo{}: {word:?}: no such subcommand", self.plain_path());
        2
    }

    /// What to print when a tool was given nothing and needs something.
    pub fn missing(&self, what: &str) -> i32 {
        eprint!("{}", self.overview(Paint::plain()));
        eprintln!("\noslo{}: {what}", self.plain_path());
        2
    }

    /// The same, for something wrong with one subcommand rather than with the choice of one.
    ///
    /// **That row's page rather than the whole list**: somebody who typed `sync` with a bad flag
    /// has already chosen the subcommand, and printing the other eight is answering a question they
    /// did not ask.
    pub fn wrong(&self, name: &str, what: &str) -> i32 {
        match self.subcommand(name, Paint::plain()) {
            Some(page) => eprint!("{page}"),
            None => eprint!("{}", self.overview(Paint::plain())),
        }
        eprintln!("\noslo{} {name}: {what}", self.plain_path());
        2
    }

    /// Refuse an operand the subcommand was never going to read.
    ///
    /// **A word a tool ignores is a mistake, not a decoration.** `oslo config files EXTRA` printed
    /// the list and reported success, so a name meant for a different subcommand — or a typo — read
    /// as though it had been acted on. The same silent acceptance `printf -Z`, `trap -z EXIT` and
    /// `ls | length extra` all had.
    ///
    /// `args` is the tool's own slice with the subcommand at `args[0]`, and `wanted` is how many
    /// operands after it the subcommand actually reads. `None` means there was nothing extra, so
    /// the caller carries on:
    ///
    /// ```ignore
    /// Some("files") => MENU.extra("files", args, 0).unwrap_or_else(files),
    /// ```
    pub fn extra(&self, name: &str, args: &[String], wanted: usize) -> Option<i32> {
        let extra = args.get(wanted + 1)?;
        Some(self.wrong(name, &describe_extra(extra)))
    }

    /// The path with each word painted as a thing you type.
    fn painted_path(&self, paint: Paint) -> String {
        self.path
            .iter()
            .map(|word| format!(" {}", paint.key(word)))
            .collect()
    }

    /// The same, for a message that carries no colour of its own.
    fn plain_path(&self) -> String {
        self.path.iter().map(|word| format!(" {word}")).collect()
    }

    /// The last word of the path: the name this menu is reached by from the one above it.
    fn leaf(&self) -> &'static str {
        self.path.last().copied().unwrap_or_default()
    }
}

impl Sub {
    /// Whether `--help` on this row would say more than the list already did.
    fn has_a_page(&self) -> bool {
        !self.flags.is_empty() || !self.note.is_empty()
    }
}

/// Wraps a note to the terminal width.
fn wrapped(note: &str) -> Vec<String> {
    const INDENT: usize = 2;
    let width = oslo::ui::dropdown::width::terminal_cols()
        .saturating_sub(INDENT + 2)
        .clamp(32, 96);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in note.split_whitespace() {
        let would_be = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
        if would_be > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
#[path = "menu/tests.rs"]
mod tests;
