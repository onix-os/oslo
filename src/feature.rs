//! Turning parts of the shell off, and on again, while it runs.
//!
//! ```lua
//! oslo.feature.set("direnv", false)                        -- now
//! oslo.feature.when("direnv", function(dir)                -- whenever the directory changes
//!   return not envrc_above(dir)
//! end)
//! ```
//!
//! # A mask, not an assignment
//!
//! **This never writes to the configuration.** `oslo.vi.enabled` says what you asked for; a feature
//! bit says whether it is in force *at this moment*, and the two are ANDed at the point of use.
//! That is the whole reason this is a separate mechanism rather than a setter on `Settings`, and it
//! buys the hard part for free: **re-enabling restores exactly what the config said**, so a handler
//! that turns something off on the way into a directory has nothing to remember and nothing to put
//! back. "Disable on entering, restore on leaving" is where that kind of code goes wrong, and there
//! is no state here to leak.
//!
//! # Why a process-global bitset
//!
//! The same answer `env::builtins::shopt` gives for its own options, plus one reason specific to
//! hooks: a feature is most useful from a directory hook, and `pre-change-dir` is an *answering*
//! hook, so it runs inline while the `cd` builtin holds the `Environment`. State kept on
//! `Environment` would be unreachable from exactly the place that wants it most. An atomic is
//! readable and writable from anywhere, on any path, holding anything.
//!
//! **Disabled** bits rather than enabled ones, so the default — nothing set, everything working —
//! is zero and needs no initialisation, and a bit that is never written can never turn something
//! off.
//!
//! # A fixed table, and a name nothing gates is a bug
//!
//! Every row here has a real gate site. A feature that could be turned off without anything reading
//! the bit is indistinguishable from a typo in the name — the same rule `lua::api::hooks` states
//! for its own table, arrived at the same way.
//!
//! # What is deliberately absent
//!
//! **History and the tracking store are not features and must never become ones.** They are what
//! the command log and the frecency table are built from, and something downstream is entitled to
//! assume the log is complete rather than "complete except where a config had an opinion". A gap
//! nobody can see is worse than no data.
//!
//! `redact` and `--profile` are the controls that exist for this, and both leave a record that is
//! honestly shaped: redaction drops a risky line's *arguments* and keeps the command, a profile
//! separates two chronologies without truncating either. There is a test below that refuses the
//! names outright, so the next person to want this meets an argument rather than an absence.

use std::sync::atomic::{AtomicU32, Ordering};

/// One thing that can be turned off, and what turning it off does.
pub struct Feature {
    /// The name Lua and every diagnostic uses.
    pub name: &'static str,
    /// Builtins that stop being builtins while this is off.
    ///
    /// The name then resolves through `$PATH` like any other word, which is the point for
    /// `direnv`: oslo's builtin cannot read an `.envrc`, and the real one can.
    pub provides: &'static [&'static str],
    /// One line, for `oslo.feature.list()`.
    pub about: &'static str,
}

/// Everything that can be turned off.
///
/// **The order is load-bearing** — the bitset indexes into it — so the constants in [`at`] are
/// checked against it by a test rather than trusted.
pub const FEATURES: &[Feature] = &[
    Feature {
        name: "direnv",
        provides: &["direnv"],
        about: "directory environments: .env.lua, and the direnv builtin",
    },
    Feature {
        name: "vi",
        provides: &[],
        about: "vi key bindings while editing a line",
    },
    Feature {
        name: "suggest",
        provides: &[],
        about: "the ghost suggestion ahead of the cursor",
    },
    Feature {
        name: "abbr",
        provides: &["abbr"],
        about: "abbreviations that expand as you type, and the abbr builtin",
    },
    Feature {
        name: "notify",
        provides: &[],
        about: "a desktop notification when a slow command finishes",
    },
    Feature {
        name: "marks",
        provides: &[],
        about: "OSC 133 semantic marks and the terminal title",
    },
    Feature {
        name: "finder",
        provides: &[],
        about: "the full-screen history finder",
    },
    Feature {
        name: "rm",
        provides: &["rm"],
        about: "the rm builtin; off means /usr/bin/rm",
    },
];

/// The features something on a hot path asks about by index.
///
/// Named for the same reason `lua::api::hooks::at` is: the check sits on the keystroke and
/// per-command paths, where walking eight names would be real work for nothing.
pub mod at {
    pub const DIRENV: usize = 0;
    pub const VI: usize = 1;
    pub const SUGGEST: usize = 2;
    pub const ABBR: usize = 3;
    pub const NOTIFY: usize = 4;
    pub const MARKS: usize = 5;
    pub const FINDER: usize = 6;
    pub const RM: usize = 7;
}

/// Which features are **off**. Bit `n` is `FEATURES[n]`. See the module note on why it is inverted.
static DISABLED: AtomicU32 = AtomicU32::new(0);

/// Whether the feature at `index` is in force.
pub fn on(index: usize) -> bool {
    DISABLED.load(Ordering::Relaxed) & (1 << index) == 0
}

/// Turn the feature at `index` on or off.
pub fn set(index: usize, enabled: bool) {
    let bit = 1 << index;
    if enabled {
        DISABLED.fetch_and(!bit, Ordering::Relaxed);
    } else {
        DISABLED.fetch_or(bit, Ordering::Relaxed);
    }
}

/// The index of the feature called `name`, or `None`.
///
/// `None` is a *refusal*, and every caller reports it. A config that turns off `direnvv` and is
/// quietly obeyed has no way to find out it did nothing.
pub fn resolve(name: &str) -> Option<usize> {
    FEATURES.iter().position(|f| f.name == name.trim())
}

/// Whether `name` is a builtin that some **disabled** feature provides.
///
/// The single question the builtin registry asks, so a disabled builtin is invisible to the
/// dispatcher, to `type`, to `command -v` and to completion all at once rather than in four places
/// that can disagree.
///
/// The common case — nothing disabled — is one relaxed load and no walk at all, which is what makes
/// this affordable on every command.
pub fn builtin_is_off(name: &str) -> bool {
    let disabled = DISABLED.load(Ordering::Relaxed);
    if disabled == 0 {
        return false;
    }
    let name = name.trim();
    FEATURES
        .iter()
        .enumerate()
        .any(|(index, feature)| disabled & (1 << index) != 0 && feature.provides.contains(&name))
}

/// Every feature, whether it is on, and what it does — for `oslo.feature.list()`.
pub fn listing() -> Vec<(&'static str, bool, &'static str)> {
    FEATURES
        .iter()
        .enumerate()
        .map(|(index, f)| (f.name, on(index), f.about))
        .collect()
}

/// Put every feature back on. For tests, which share one process and therefore one bitset.
#[cfg(test)]
pub(crate) fn reset() {
    DISABLED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The named indices are what the table says they are. A constant pointing at the wrong row
    /// would turn off a different feature than the one asked for, silently.
    #[test]
    fn the_named_indices_match_the_table() {
        for (index, name) in [
            (at::DIRENV, "direnv"),
            (at::VI, "vi"),
            (at::SUGGEST, "suggest"),
            (at::ABBR, "abbr"),
            (at::NOTIFY, "notify"),
            (at::MARKS, "marks"),
            (at::FINDER, "finder"),
            (at::RM, "rm"),
        ] {
            assert_eq!(
                FEATURES[index].name, name,
                "at::* is out of step with FEATURES"
            );
        }
        assert!(
            FEATURES.len() <= 32,
            "the bitset is a u32; a 33rd feature needs a wider one"
        );
    }

    /// **Recording is not a feature.** See the module note: the command log has to be complete, or
    /// what reads it cannot tell a quiet shell from a config that turned recording off.
    ///
    /// A test rather than an absence, because an absence is not an argument and the next person to
    /// want this will not read the comment first.
    #[test]
    fn recording_can_never_be_turned_off() {
        for forbidden in ["history", "tracking", "track", "log", "frecency", "record"] {
            assert!(
                resolve(forbidden).is_none(),
                "{forbidden} must not be a feature: the store it names is what a predictor reads, \
                 and a gap in it is invisible to everything downstream. Redaction and --profile \
                 are the controls for this; both leave a record that is honestly shaped."
            );
        }
    }

    /// Names are unique, or `resolve` would answer about whichever came first.
    #[test]
    fn every_name_is_distinct() {
        let mut seen = std::collections::HashSet::new();
        for feature in FEATURES {
            assert!(
                seen.insert(feature.name),
                "{} is listed twice",
                feature.name
            );
        }
    }

    /// A builtin belongs to exactly one feature, or turning one off would leave the other's promise
    /// unkept while its bit still said it was on.
    #[test]
    fn a_builtin_is_provided_by_one_feature() {
        let mut seen = std::collections::HashSet::new();
        for feature in FEATURES {
            for name in feature.provides {
                assert!(seen.insert(*name), "{name} is provided by two features");
            }
        }
    }

    /// Every name resolves, and an invented one does not.
    #[test]
    fn a_name_that_is_not_a_feature_is_refused() {
        for feature in FEATURES {
            assert_eq!(resolve(feature.name), resolve(feature.name));
            assert!(resolve(feature.name).is_some());
        }
        assert_eq!(resolve("direnvv"), None);
        assert_eq!(resolve(""), None);
    }
}
