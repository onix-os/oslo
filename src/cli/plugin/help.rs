//! What `oslo plugin --help` says.
//!
//! The rows only; how they are drawn is [`crate::cli::help::menu`]'s, so this page and every other
//! tool's are one page with different words in it.

use crate::cli::help::Paint;
use crate::cli::help::menu::{CALL, Menu, SUBCOMMANDS as HEADING, Sub};

pub(crate) const MENU: Menu = Menu {
    path: &["plugin"],
    call: CALL,
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &[
        "A plugin's commands run in the interactive shell, never in a script.",
        "`oslo --noplugin` starts with none of them, which is how you answer \"is it me or a plugin?\".",
    ],
    nested: &[],
};

pub(super) const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "list",
        args: "",
        about: "the runtimepath, and the plugins on it, in load order",
        flags: &[],
        note: "There is no install verb: a plugin is a directory on the path, so installing one is \
               putting it in ~/.local/share/oslo/site/pack/<any>/start/, and removing one is taking \
               it away. What is there runs, because you put it there.",
    },
    Sub {
        name: "doctor",
        args: "[NAME]",
        about: "check the path, and why a plugin might not be working",
        flags: &[],
        note: "With a name, the plugin is loaded and its own checks are asked too — the only ones \
               that know whether the program it shells out to is installed. Without one, nothing is \
               loaded.",
    },
    Sub {
        name: "test",
        args: "[DIRECTORY]",
        about: "run a plugin's own assertions, in a home with nothing in it",
        flags: &[],
        note: "The directory defaults to the current one, so an author runs this from inside what \
               they are writing. $HOME, $XDG_DATA_HOME and $XDG_CONFIG_HOME point at a temporary \
               directory for the run — a test that passes because your own database already has \
               something in it is the failure a user hits on day one. A plugin declares its tests \
               with `oslo.plugin.test`.",
    },
];

/// The overview.
pub fn text(paint: Paint) -> String {
    MENU.overview(paint)
}

#[cfg(test)]
mod tests;
