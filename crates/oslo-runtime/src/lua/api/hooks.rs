//! `oslo.on` — every moment a config can attach to.
//!
//! # One event, several spellings
//!
//! A hook has a **canonical** name in the `pre-`/`post-`/`on-` scheme, and may answer to older
//! ones. `oslo.on.preexec(f)` and `oslo.on["pre-cmd"](f)` attach to the same list and fire
//! together, because they are the same moment — the older spellings shipped first and configs were
//! written against them.
//!
//! Kebab-case cannot be written as a Lua field (`oslo.on.pre-cmd` is a subtraction), so every
//! canonical name is also installed with underscores: `oslo.on.pre_cmd` is `oslo.on["pre-cmd"]`.
//! Both are real; neither is preferred.
//!
//! # A fixed list, still
//!
//! A name nothing fires is indistinguishable from a typo, and `oslo.on.precmb(f)` doing nothing
//! for ever is the failure this avoids. Adding a moment means adding a row here *and* a fire site;
//! a row on its own is caught by the test that walks them.
//!
//! # What a hook may answer
//!
//! Most are told something and their return value is discarded. Three are asked a question:
//!
//! * `pre-cmd` may return a replacement line, or `false` to cancel the command.
//! * `pre-change-dir` may return `false` to refuse the move.
//! * `on-command-not-found` may return a status, meaning it handled the situation.
//!
//! Everything else observes. That is not timidity: there is nothing coherent for `post-prompt` to
//! veto, and a `pre-mode-change` that could refuse would let a config make vi mode inescapable.

/// One moment, and the spellings that reach it.
pub struct Hook {
    /// The `pre-`/`post-`/`on-` name. The one the registry keys on.
    pub name: &'static str,
    /// Names that mean the same moment. Older spellings, kept so no config breaks.
    pub aliases: &'static [&'static str],
    /// Whether a handler's return value is read. See the module docs.
    pub answers: bool,
}

/// Every moment, in the order their `watched` bits are numbered.
///
/// **The order is load-bearing** — the watched bitset indexes into it — so the constants below are checked
/// against it by a test rather than trusted.
pub const HOOKS: &[Hook] = &[
    Hook {
        name: "pre-cmd",
        aliases: &["preexec", "precmd"],
        answers: true,
    },
    Hook {
        name: "post-cmd",
        aliases: &["postexec", "postcmd"],
        answers: false,
    },
    Hook {
        name: "pre-change-dir",
        aliases: &[],
        answers: true,
    },
    // `cd` was the only directory hook, and it fired *after* the move — so it is this one, not
    // `pre-change-dir`, that inherits the name.
    Hook {
        name: "post-change-dir",
        aliases: &["cd"],
        answers: false,
    },
    Hook {
        name: "pre-prompt",
        aliases: &["prompt"],
        answers: false,
    },
    Hook {
        name: "post-prompt",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "pre-mode-change",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "post-mode-change",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-history-open",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-history-close",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-history-select",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-completion-start",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-completion-cancel",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-completion-select",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-job-finish",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-time-report",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-command-not-found",
        aliases: &["command-not-found"],
        answers: true,
    },
    Hook {
        name: "on-idle-timeout",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-report",
        aliases: &[],
        answers: true,
    },
    Hook {
        name: "pre-record",
        aliases: &[],
        answers: true,
    },
    Hook {
        name: "on-exit",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-key",
        aliases: &["key"],
        answers: true,
    },
    Hook {
        name: "on-secret-encrypt",
        aliases: &[],
        answers: true,
    },
    Hook {
        name: "on-secret-decrypt",
        aliases: &[],
        answers: true,
    },
    Hook {
        name: "pre-secret-access",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "post-secret-access",
        aliases: &[],
        answers: false,
    },
    // The two OS-lifecycle observers. Notification only: a handler watching a child end has
    // nothing to answer, and letting one veto a process that has *already exited* would be a
    // promise the kernel does not keep.
    Hook {
        name: "on-process-exit",
        aliases: &[],
        answers: false,
    },
    Hook {
        name: "on-job-state",
        aliases: &[],
        answers: false,
    },
];

/// The moments something on a hot path has to ask about, by index into [`HOOKS`].
///
/// **One set of indices, kept where the shell can see them.** These live in [`oslo_base::hooks`], which
/// depends on nothing, because the editor and the executor fire hooks and must not have to see this
/// module to do it. Re-exported here so that the names and the table they index into still read
/// together, and so `HOOKS` remains the thing they are checked against — see the test below.
pub use oslo_base::hooks::at;

/// Which hooks have ever had a handler attached.
///
/// **This exists so that not using a hook costs nothing.** `on-key` fires on every keystroke and
/// `post-mode-change` on every Esc; asking the registry each time — a string format, a map lookup,
/// a `Vec` — would put that on the path of simply typing. One relaxed load answers it instead.
///
/// Never cleared. Removing the last handler leaves the bit set and the lookup then finds an empty
/// list, which costs a little on a session that attached and detached; clearing it correctly would
/// mean counting handlers across config reloads, and a wrong count here silently kills the hook.
struct Watch(std::sync::atomic::AtomicU32);
static WATCHED: Watch = Watch(std::sync::atomic::AtomicU32::new(0));

/// Whether anything is attached to the hook at `index`. See the bitset note above.
pub fn watched(index: usize) -> bool {
    WATCHED.0.load(std::sync::atomic::Ordering::Relaxed) & (1 << index) != 0
}

fn mark_watched(index: usize) {
    WATCHED
        .0
        .fetch_or(1 << index, std::sync::atomic::Ordering::Relaxed);
}

/// The canonical name a spelling reaches, and its index.
pub fn resolve(spelling: &str) -> Option<(usize, &'static str)> {
    // Underscores are the field-access spelling: `oslo.on.pre_cmd` cannot be written with a dash.
    let wanted = spelling.replace('_', "-");
    HOOKS.iter().enumerate().find_map(|(index, hook)| {
        let matched = hook.name == wanted || hook.aliases.contains(&spelling);
        matched.then_some((index, hook.name))
    })
}

/// Every spelling `oslo.on` answers to: canonical, underscored, and every alias.
///
/// Built once and kept, because the setters installed on `oslo.on` outlive the call that made
/// them — and because a config reload builds a fresh `oslo` table, so computing them each time
/// would leak a little more on every reload.
pub fn spellings() -> &'static [(&'static str, usize)] {
    static ALL: std::sync::OnceLock<Vec<(&'static str, usize)>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        let mut names = Vec::new();
        for (index, hook) in HOOKS.iter().enumerate() {
            names.push((hook.name, index));
            let underscored = hook.name.replace('-', "_");
            if underscored != hook.name {
                names.push((&*String::leak(underscored), index));
            }
            for alias in hook.aliases {
                names.push((*alias, index));
            }
        }
        names
    })
}

/// Record that `index` now has a handler, so the hot-path check can find it.
pub fn attached(index: usize) {
    mark_watched(index);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The named indices are what the table says they are. `at::ON_KEY` pointing at the wrong row
    /// would make the keystroke check answer about a different hook entirely — silently.
    #[test]
    fn the_named_indices_match_the_table() {
        for (index, name) in [
            (at::PRE_CMD, "pre-cmd"),
            (at::POST_CMD, "post-cmd"),
            (at::PRE_CHANGE_DIR, "pre-change-dir"),
            (at::POST_CHANGE_DIR, "post-change-dir"),
            (at::PRE_PROMPT, "pre-prompt"),
            (at::POST_PROMPT, "post-prompt"),
            (at::PRE_MODE_CHANGE, "pre-mode-change"),
            (at::POST_MODE_CHANGE, "post-mode-change"),
            (at::HISTORY_OPEN, "on-history-open"),
            (at::HISTORY_CLOSE, "on-history-close"),
            (at::HISTORY_SELECT, "on-history-select"),
            (at::COMPLETION_START, "on-completion-start"),
            (at::COMPLETION_CANCEL, "on-completion-cancel"),
            (at::COMPLETION_SELECT, "on-completion-select"),
            (at::JOB_FINISH, "on-job-finish"),
            (at::PROCESS_EXIT, "on-process-exit"),
            (at::JOB_STATE, "on-job-state"),
            (at::TIME_REPORT, "on-time-report"),
            (at::COMMAND_NOT_FOUND, "on-command-not-found"),
            (at::IDLE_TIMEOUT, "on-idle-timeout"),
            (at::ON_REPORT, "on-report"),
            (at::PRE_RECORD, "pre-record"),
            (at::ON_EXIT, "on-exit"),
            (at::ON_KEY, "on-key"),
            (at::SECRET_ENCRYPT, "on-secret-encrypt"),
            (at::SECRET_DECRYPT, "on-secret-decrypt"),
            (at::PRE_SECRET_ACCESS, "pre-secret-access"),
            (at::POST_SECRET_ACCESS, "post-secret-access"),
        ] {
            assert_eq!(HOOKS[index].name, name, "at::* is out of step with HOOKS");
        }
        assert!(
            HOOKS.len() <= 32,
            "the watched bitset is a u32; a 33rd hook needs a wider one"
        );
    }

    /// Every spelling reaches its moment, and none reaches two.
    #[test]
    fn every_spelling_resolves_to_one_hook() {
        for &(spelling, index) in spellings() {
            assert_eq!(
                resolve(spelling).map(|(i, _)| i),
                Some(index),
                "{spelling} does not resolve to the hook that offered it"
            );
        }
        let mut seen = std::collections::HashSet::new();
        for &(spelling, _) in spellings() {
            assert!(seen.insert(spelling), "{spelling} is listed twice");
        }
    }

    /// The old names still work, and land on the same list as the new ones. A config written
    /// against `preexec` must not quietly stop firing because the canonical name changed.
    #[test]
    fn the_older_spellings_reach_the_same_moment() {
        for (old, new) in [
            ("preexec", "pre-cmd"),
            ("precmd", "pre-cmd"),
            ("postexec", "post-cmd"),
            ("postcmd", "post-cmd"),
            ("cd", "post-change-dir"),
            ("prompt", "pre-prompt"),
            ("command-not-found", "on-command-not-found"),
            ("key", "on-key"),
        ] {
            assert_eq!(resolve(old).map(|(_, n)| n), Some(new), "{old}");
        }
        // And the underscore spelling, which is the only one a Lua field access can write.
        assert_eq!(resolve("pre_cmd").map(|(_, n)| n), Some("pre-cmd"));
        assert_eq!(
            resolve("on_history_select").map(|(_, n)| n),
            Some("on-history-select")
        );
        assert_eq!(resolve("nonsense"), None);
    }
}
