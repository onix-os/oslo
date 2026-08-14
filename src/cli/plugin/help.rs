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
    notes: &["A plugin's commands run in the interactive shell, never in a script."],
    nested: &[],
};

pub(super) const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "list",
        args: "",
        about: "what is installed, and whether it still matches what you allowed",
        flags: &[],
        note: "A plugin whose files have changed since you allowed them is marked CHANGED and \
               will not load until `oslo plugin allow` records the new hash.",
    },
    Sub {
        name: "doctor",
        args: "[NAME]",
        about: "check what is installed, and why a plugin might not be working",
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
               something in it is the failure a user hits on day one. The plugin is loaded straight \
               out of the directory without a trust check, which is the same trust as running a \
               script you just wrote. A plugin declares its tests with `oslo.plugin.test`.",
    },
    Sub {
        name: "install",
        args: "PATH|GIT [--yes]",
        about: "copy or clone a plugin in, after showing what it reserves",
        flags: &[(
            "--yes",
            "do not ask before allowing it to run, for a script",
        )],
        note: "A git source must name a revision — `github:user/repo@v1.0` — because a branch \
               would be a different plugin tomorrow and the trust hash would refuse it every \
               morning. Nothing of the plugin runs during an install: its manifest is read in an \
               interpreter that cannot reach the shell, and the names it would reserve are shown \
               before you decide.",
    },
    Sub {
        name: "remove",
        args: "NAME",
        about: "delete a plugin; its database is left where it is",
        flags: &[],
        note: "The database is your data, not the plugin's. Reinstalling hands the same one back.",
    },
    Sub {
        name: "allow",
        args: "NAME",
        about: "record what a plugin hashes to now, after an update",
        flags: &[],
        note: "Look at what changed first. An update is somebody else's new code arriving on your \
               machine, which is the whole reason it stopped loading.",
    },
];

/// The overview.
pub fn text(paint: Paint) -> String {
    MENU.overview(paint)
}

#[cfg(test)]
mod tests;
