//! `oslo scratch` — the help for it, in the shape every other tool's is.
//!
//! The work is `oslo_shell`'s; this is the overview page, written here so that `oslo scratch
//! --help` and `oslo history --help` read as two pages of one manual rather than as two programs
//! that ended up in the same binary.

use crate::cli::help::{Paint, row};
use std::fmt::Write as _;

/// One thing `scratch` can be asked.
struct Sub {
    name: &'static str,
    about: &'static str,
}

/// **Operands rather than subcommands**, which is the one way this differs from `history` — a
/// scratch is named, and a name cannot be a reserved word without taking it away from somebody.
const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "<none>",
        about: "open the finder — the same one the key opens",
    },
    Sub {
        name: "NAME",
        about: "go into that scratch, making it if it is not running",
    },
    Sub {
        name: "-l",
        about: "every scratch, one per line",
    },
];

/// The overview page.
pub fn text(paint: Paint) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "{}", paint.head("USAGE"));
    let _ = writeln!(
        text,
        "  {} {} {}",
        paint.key("oslo"),
        paint.key("scratch"),
        paint.slot("[name | -l]")
    );
    let _ = writeln!(text, "\n{}", paint.head("OPERANDS"));
    for sub in SUBCOMMANDS {
        text.push_str(&row(sub.name, paint.key(sub.name), sub.about));
    }
    let _ = writeln!(
        text,
        "\n  {}",
        paint.dim("`$SCRATCH` is the one you are in, for a prompt to read.")
    );
    text
}
