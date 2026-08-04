//! `--help`, and `--help --details`.
//!
//! # Colour is detected, never assumed
//!
//! Two questions, both of which have to say yes:
//!
//! - [`Depth::detect`] — `$NO_COLOR`, a `dumb` or empty `$TERM`, and how much colour the terminal
//!   admits to. A serial console gets 16 colours rather than nothing, which for a distro's
//!   `/bin/sh` is not a hypothetical.
//! - **is stdout a terminal** — `oslo --help | less` and `oslo --help > FILE` must not have
//!   escapes in them. `Depth::detect` cannot answer this: it reads the environment, and the
//!   environment is the same either way.
//!
//! There is no `--color` flag and no plain-help spelling. `NO_COLOR=1 oslo --help` is the
//! convention that already exists for this, and piping already does the right thing.
//!
//! # Why the reference is a separate view
//!
//! `--help` that printed all twenty-two shell options would bury the six things somebody running
//! it actually wanted. `--details` is where the long form lives, and the short form says so.

use crate::cli::tools::TOOLS;
use oslo::env::options::ALL;
use oslo::interactive::theme::{Color, Depth, Style};
use std::fmt::Write as _;

/// How this run should be painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paint {
    depth: Depth,
}

impl Paint {
    /// What the terminal on the other end of stdout can take.
    pub fn detect() -> Paint {
        let depth = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            Depth::detect()
        } else {
            // A pipe or a file. Escapes here are not colour, they are litter in somebody's
            // `grep`, and `$TERM` says nothing about it either way.
            Depth::None
        };
        Paint { depth }
    }

    /// No colour at all. Tests only — a real run always asks [`Paint::detect`], which reaches the
    /// same answer for a pipe without anybody having to remember to.
    #[cfg(test)]
    pub fn plain() -> Paint {
        Paint { depth: Depth::None }
    }

    fn paint(self, text: &str, style: Style) -> String {
        style.paint(text, self.depth)
    }

    /// A section heading.
    pub fn head(self, text: &str) -> String {
        self.paint(
            text,
            Style {
                bold: true,
                ..Style::fg(Color::Indexed(3))
            },
        )
    }

    /// A flag, a tool name, an option letter — the thing you type.
    pub fn key(self, text: &str) -> String {
        self.paint(text, Style::fg(Color::Indexed(6)))
    }

    /// A placeholder like `COMMAND` or `NAME`, which is not typed literally.
    pub fn slot(self, text: &str) -> String {
        self.paint(
            text,
            Style {
                italic: true,
                ..Style::fg(Color::Indexed(5))
            },
        )
    }

    /// An aside: the `(inactive)` mark, the note under a section.
    pub fn dim(self, text: &str) -> String {
        self.paint(text, Style::fg(Color::Indexed(8)))
    }
}

/// The width the description column starts at. Wide enough for `--profile=NAME`.
const COLUMN: usize = 20;

/// One `flag  description` line, with the description aligned.
///
/// Padded on the *unpainted* width. Escapes have no width on screen but plenty in a `String`, so
/// padding the painted text would leave every coloured column ragged.
///
/// **The description is left unpainted.** It is the text somebody is here to read; dimming it
/// makes the whole page grey on a terminal that renders colour 8 close to the background, and
/// grey-on-grey is the exact complaint that got bold removed from the finder's matches. Colour is
/// for the thing you type and for asides — not for the body.
fn row(key: &str, painted_key: String, about: &str) -> String {
    let pad = COLUMN.saturating_sub(key.chars().count());
    format!("  {}{}{}\n", painted_key, " ".repeat(pad), about)
}

/// The two invocation forms.
fn synopsis(paint: Paint) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{}", paint.head("USAGE"));
    let _ = writeln!(
        s,
        "  {} {} {}",
        paint.key("oslo"),
        paint.slot("[option]..."),
        paint.slot("[script [argument]...]")
    );
    let _ = writeln!(
        s,
        "  {} {} {} {} {}",
        paint.key("oslo"),
        paint.slot("[option]..."),
        paint.key("-c"),
        paint.slot("command"),
        paint.slot("[name [argument]...]")
    );
    let _ = writeln!(
        s,
        "  {} {} {}",
        paint.key("oslo"),
        paint.slot("<tool>"),
        paint.slot("[argument]...")
    );
    s
}

/// The short help: what somebody running `oslo --help` came for.
pub fn short(paint: Paint) -> String {
    let mut s = synopsis(paint);

    let _ = writeln!(s, "\n{}", paint.head("OPTIONS"));
    for (key, about) in INVOCATION {
        s.push_str(&row(key, paint.key(key), about));
    }
    s.push_str(&row(
        "-e -u -x ...",
        format!(
            "{} {} {} {}",
            paint.key("-e"),
            paint.key("-u"),
            paint.key("-x"),
            paint.dim("...")
        ),
        "set a shell option, as `set` does",
    ));
    for (key, about) in LONG {
        let painted = match key.split_once('=') {
            Some((flag, slot)) => format!("{}={}", paint.key(flag), paint.slot(slot)),
            None => paint.key(key),
        };
        s.push_str(&row(key, painted, about));
    }

    s.push_str(&tools_section(paint));

    let _ = write!(
        s,
        "\n{}\n",
        paint.dim("`oslo --help --details` lists every shell option and what it does.")
    );
    s
}

/// The flags that are not `set` options and take no long form.
const INVOCATION: &[(&str, &str)] = &[
    ("-c COMMAND", "run COMMAND, then exit"),
    ("-s", "read commands from standard input"),
    ("-i", "force interactive mode"),
    ("-l, --login", "act as a login shell"),
];

/// oslo's own flags. Long-only by design: the short letters belong to POSIX, which claimed the
/// whole alphabet in 1988 — `-h` is `hashall` and `-v` is `verbose`, which is why bash has no
/// `-h`/`-v` either.
///
/// **`--posix` is deliberately not here.** It still works, and the differential suite needs it to
/// run the corpus both ways — but nobody should reach for it, because a shell invoked as `sh`
/// already is one and every other invocation is oslo being itself. Listing it would advertise a
/// choice that is already made. `--help --details` documents `posix` under the shell options,
/// where somebody looking for it will find it.
const LONG: &[(&str, &str)] = &[
    ("--details", "with --help: the full option reference"),
    ("--version", "print the version, then exit"),
    ("--help", "print this message, then exit"),
    ("--", "end of options"),
];

/// The tools.
///
/// Nothing to install and nothing to mark: a tool is reachable the moment the binary is.
fn tools_section(paint: Paint) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "\n{}  {}",
        paint.head("TOOLS"),
        paint.dim("a script of the same name always wins")
    );
    for tool in TOOLS {
        s.push_str(&row(tool.name, paint.key(tool.name), tool.about));
    }
    s
}

/// `--help --details`: every shell option, what it does, and whether oslo implements it.
pub fn details(paint: Paint) -> String {
    let mut s = short(paint);

    let _ = writeln!(
        s,
        "\n{}  {}",
        paint.head("SHELL OPTIONS"),
        paint.dim("set -x / set -o xtrace / set +x to turn off")
    );

    for option in ALL {
        // The invocation flags are reported by `$-` but `set` cannot change them, so they are not
        // options anybody can pass here — they appear under their own heading below.
        if !option.settable() {
            continue;
        }
        let letter = option
            .letter
            .map_or_else(|| "  ".to_string(), |c| format!("-{c}"));
        let name = option.name.unwrap_or_default();
        let key = format!("{letter} {name}");
        let pad = COLUMN.saturating_sub(key.chars().count());
        let _ = writeln!(
            s,
            "  {} {}{}{}",
            paint.key(&letter),
            paint.key(name),
            " ".repeat(pad),
            option.about
        );
        // **Said here, not discovered by running it.** An option oslo accepts and does not act on
        // is the one thing a script cannot detect, so the reference is where it gets admitted.
        if let Some(why) = option.unsupported {
            let _ = writeln!(
                s,
                "  {}{}",
                " ".repeat(COLUMN + 1),
                paint.dim(&format!("not implemented: {why}"))
            );
        }
    }

    let _ = writeln!(
        s,
        "\n{}  {}",
        paint.head("INVOCATION FLAGS"),
        paint.dim("reported by $-; `set` cannot change them")
    );
    for option in ALL {
        if option.settable() {
            continue;
        }
        let letter = option.letter.map_or_else(String::new, |c| format!("-{c}"));
        s.push_str(&row(&letter, paint.key(&letter), option.about));
    }
    s
}

#[cfg(test)]
#[path = "help/tests.rs"]
mod tests;
