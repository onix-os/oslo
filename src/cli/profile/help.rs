//! What `oslo profile --help` says.
//!
//! The rows only; how they are drawn is [`crate::cli::help::menu`]'s, so this page and every other
//! tool's are one page with different words in it.

use crate::cli::help::menu::{CALL, Menu, SUBCOMMANDS as HEADING, Sub};

pub(crate) const MENU: Menu = Menu {
    path: &["profile"],
    call: CALL,
    heading: HEADING,
    subs: SUBCOMMANDS,
    notes: &["A bare `oslo profile` lists them, marking the one this shell is using."],
    nested: &[&KEY],
};

const SUBCOMMANDS: &[Sub] = &[
    Sub {
        name: "list",
        args: "",
        about: "every profile, marking the one in use",
        flags: &[],
        note: "A profile with no key is not broken — it simply cannot be synced yet, which is why \
               the list says so beside the name rather than leaving the column blank.",
    },
    Sub {
        name: "show",
        args: "[NAME]",
        about: "where its files are, and its fingerprint",
        flags: &[],
        note: "Two directories with opposite rules: the store under $XDG_DATA_HOME is the thing \
               that travels, and the key under $XDG_STATE_HOME is the thing that must not. A key \
               kept beside the store would be copied by every backup and every sync.",
    },
    // A row that leads to a menu of its own says only what it is: `--help` on it answers with
    // [`KEY`], and anything written twice is a thing that can disagree with itself.
    Sub {
        name: "key",
        args: "init|path [NAME]",
        about: "give a profile a key, so it can be synced",
        flags: &[],
        note: "",
    },
    Sub {
        name: "export",
        args: "[NAME]",
        about: "the key, to carry to another machine",
        flags: &[],
        note: "To standard output and nothing else — no file is written and no path is printed \
               beside it, so `oslo profile export | ssh other oslo profile import` is one line and \
               leaves nothing behind on either end.",
    },
    Sub {
        name: "import",
        args: "[NAME]",
        about: "read one from standard input",
        flags: &[],
        note: "Replacing whatever was there, because that is what importing means: the machine \
               that already has the profile is the authority, and this one is joining it.",
    },
    Sub {
        name: "sync",
        args: "USER@HOST [NAME] [--dry-run]",
        about: "two-way sync with the oslo over there",
        flags: &[
            ("--dry-run", "say what would change and change nothing"),
            ("$OSLO_SSH", "how to get there, if a bare `ssh` is not it"),
            ("$OSLO_SSH_REMOTE_BIN", "what `oslo` is called over there"),
        ],
        note: "Both ends end up with the union, and the far end is oslo rather than `scp` — a \
               store is a live database, and copying the file under a shell that is writing to it \
               is how you get half a transaction. It refuses unless both ends hold the same \
               profile key, because `default` here and `default` on a box you have an account on \
               are two histories that share a word. A command run on both machines is not a \
               conflict: every event carries the host that ran it, so both survive.",
    },
    Sub {
        name: "fingerprint",
        args: "[NAME]",
        about: "what the two ends of a sync compare",
        flags: &[],
        note: "A hash of the key, never the key: this is the half that crosses the wire, and it \
               gives nothing away. Sixteen hex characters, short enough to read down a phone.",
    },
    Sub {
        name: "send",
        args: "[NAME]",
        about: "a snapshot of this profile's store, on standard output",
        flags: &[],
        note: "The far end's half of a sync, run over ssh rather than typed. A consistent \
               snapshot, not a file copy.",
    },
    Sub {
        name: "receive",
        args: "[NAME]",
        about: "merge a snapshot arriving on standard input",
        flags: &[],
        note: "The other half, and it merges rather than replaces — between the moment the other \
               end asked for a copy and the moment it hands one back, a shell here may have \
               recorded something.",
    },
];

/// `oslo profile key`, which is its own small menu.
pub(crate) const KEY: Menu = Menu {
    path: &["profile", "key"],
    call: CALL,
    heading: HEADING,
    subs: KEYS,
    notes: &[
        "The key is in $XDG_STATE_HOME, never in the store — the store is what travels.",
        "To share a profile, export this key to the other machine rather than making a second.",
    ],
    nested: &[],
};

const KEYS: &[Sub] = &[
    Sub {
        name: "init",
        args: "[NAME]",
        about: "make one, and print its fingerprint",
        flags: &[],
        note: "Refuses when there is already one, rather than replacing it. A machine joining an \
               existing profile imports that profile's key instead of making its own.",
    },
    Sub {
        name: "path",
        args: "[NAME]",
        about: "where the key is, without reading it",
        flags: &[],
        note: "Under $XDG_STATE_HOME, mode 0600, and never beside the store.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::help::Paint;

    /// The nested menu says the same things as the row that leads to it, or one of them is wrong.
    #[test]
    fn the_key_row_and_the_key_menu_agree() {
        let row = MENU
            .subcommand("key", Paint::plain())
            .expect("key is listed");
        for sub in KEYS {
            assert!(row.contains(sub.name), "{} is not in the row", sub.name);
        }
        let nested = KEY.overview(Paint::plain());
        assert!(nested.contains("oslo profile key"), "{nested}");
    }
}
