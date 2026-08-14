//! What `oslo sync --help` says.
//!
//! The rows only; how they are drawn is [`crate::cli::help::menu`]'s, so this page and every other
//! tool's are one page with different words in it.

use crate::cli::help::menu::{Menu, SUBCOMMANDS as HEADING, Sub};

pub(crate) const MENU: Menu = Menu {
    path: &["sync"],
    call: "USER@HOST [--only WHAT] [--dry-run]",
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &[
        "Both ends come out with the union, so it does not matter which machine you type it on.",
        "Refuses unless both hold the same profile key: `oslo profile export | ssh HOST oslo",
        "profile import` pairs them, once.",
        "$OSLO_SSH is how to get there when a bare `ssh` is not it; $OSLO_SSH_REMOTE_BIN is what",
        "`oslo` is called over there.",
    ],
    nested: &[],
};

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "--only",
        args: "history | macros | secrets",
        about: "sync one part instead of all three",
        flags: &[
            ("history", "the commands this profile remembers"),
            ("macros", "aliases, abbreviations, functions, scripts, vars"),
            ("secrets", "the store, carried sealed and never opened"),
        ],
        note: "Repeat it for two of the three. Without it everything travels, which is what \
               somebody who typed `oslo sync` meant. History is per profile; macros and secrets \
               are one per machine, so those two are the same wherever you sync from.",
    },
    Sub {
        name: "--dry-run",
        args: "",
        about: "say what would move and move nothing",
        flags: &[],
        note: "Neither machine is written to. The far end is still asked for its copy, because \
               there is no way to say what would change without seeing what is over there.",
    },
    Sub {
        name: "send",
        args: "WHAT [PROFILE]",
        about: "this machine's copy of one part, on standard output",
        flags: &[],
        note: "The far end's half, run over ssh rather than typed. A consistent snapshot rather \
               than a file copy: these are live databases, and copying one under a shell that is \
               writing to it is how you get half a transaction.",
    },
    Sub {
        name: "receive",
        args: "WHAT [PROFILE]",
        about: "merge a copy arriving on standard input",
        flags: &[],
        note: "The other half, and it merges rather than replaces — between the moment this \
               machine handed over its copy and the moment the merged one comes back, something \
               here may have changed.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::help::Paint;

    /// The three parts named in the page are the three the parser accepts, or the help is wrong.
    #[test]
    fn the_page_names_exactly_the_parts_that_exist() {
        let page = MENU
            .subcommand("--only", Paint::plain())
            .expect("--only is listed");
        for part in super::super::part::every() {
            assert!(
                page.contains(part.word()),
                "{} is undocumented",
                part.word()
            );
            assert!(
                super::super::part::named(part.word()).is_some(),
                "{} is documented and unreachable",
                part.word()
            );
        }
    }
}
