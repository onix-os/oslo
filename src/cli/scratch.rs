//! `oslo scratch` — the help for it, in the shape every other tool's is.
//!
//! The work is `oslo_shell`'s; this is the overview page, written here so that `oslo scratch
//! --help` and `oslo history --help` read as two pages of one manual rather than as two programs
//! that ended up in the same binary.

use crate::cli::help::Paint;
use crate::cli::help::menu::{Menu, OPERANDS, Sub};

pub(crate) const MENU: Menu = Menu {
    path: &["scratch"],
    call: "[name | -l]",
    // **Operands rather than subcommands**, which is the one way this differs from `history` — a
    // scratch is named, and a name cannot be a reserved word without taking it away from somebody.
    heading: OPERANDS,
    subs: SUBCOMMANDS,
    notes: &["`$SCRATCH` is the one you are in, for a prompt to read."],
    nested: &[],
};

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "<none>",
        args: "",
        about: "open the finder — the same one the key opens",
        flags: &[],
        note: "",
    },
    Sub {
        name: "NAME",
        args: "",
        about: "go into that scratch, making it if it is not running",
        flags: &[],
        note: "",
    },
    Sub {
        name: "-l",
        args: "",
        about: "every scratch, one per line",
        flags: &[],
        note: "",
    },
];

/// The overview page.
pub fn text(paint: Paint) -> String {
    MENU.overview(paint)
}
