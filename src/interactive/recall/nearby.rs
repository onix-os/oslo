//! What you have run *here*, and what you have run anywhere in this worktree.
//!
//! The two questions the store can answer that a flat history cannot, and the one place in oslo
//! where a suggestion knows which project it is being typed in. Both are prefix searches over
//! `run`, keyed by the language as well as the directory, so a line learnt at a shell prompt is no
//! more offered at a Lua one from here than it is from the remembered set.
//!
//! # The two halves cost wildly different amounts, and it decides the design
//!
//! Measured in release against a 25,000-row, 3,000-directory store — the size the store is designed
//! to reach in about a year:
//!
//! | question | index | measured |
//! |---|---|---|
//! | this directory, `cargo run --ex` | `run(dir_id, mode, argv)` | 33 µs |
//! | this worktree, `cargo run --ex` | `run(mode, argv)` then filter | 1.8 ms |
//! | this worktree, `c` | `run(mode, argv)` then filter | 7.1 ms |
//!
//! Naming the directory pins the range to the handful of lines run in it. Widening to the worktree
//! cannot: the range is over every directory at once and the worktree filter is applied to whatever
//! comes back, so the cost is the number of rows *anywhere* that start with what has been typed —
//! worst at the first keystroke, when that is nearly everything. Typing one 26-character line cost
//! **69 ms** of database work, which is a stall on every key.
//!
//! So the exact-directory question is asked outright every time, and the widened one is remembered.
//! See [`remembered_answer`] for why remembering it is sound and [`forget_answers_for`] for the one
//! event that can make it wrong. With the memo the same line cost 13 ms cold, and 0.8 ms — 86 µs on
//! the slowest keystroke — once it had been run and offered back.

use crate::interactive::prompt;
use crate::track::Track;
use std::path::Path;
use std::sync::Mutex;

/// Where the shell is standing, and the worktree that contains it.
///
/// Both are read exactly as the write path reads them — `getcwd()`, and `prompt::git_root_of` on
/// what it answered — because a suggestion keyed on a path the store spells differently is a
/// suggestion that never appears. `startup::tracking` is the other end of that agreement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Place {
    pub(super) cwd: String,
    pub(super) root: Option<String>,
}

/// The last place resolved, so the walk up the tree happens once per directory.
///
/// The `getcwd()` is per keystroke and is one syscall; the git-root walk is a `stat` per level of
/// the path and is not, which is the whole reason this cache exists. `cwd` is its own invalidation:
/// changing directory changes the key, and nothing else can make the answer wrong.
static PLACE: Mutex<Option<Place>> = Mutex::new(None);

pub(super) fn place() -> Option<Place> {
    let cwd = std::env::current_dir().ok()?.to_string_lossy().into_owned();
    let mut slot = PLACE.lock().ok()?;
    if let Some(known) = slot.as_ref()
        && known.cwd == cwd
    {
        return Some(known.clone());
    }
    // `git_root_of` on the path already in hand rather than `git_root`, which would go and ask the
    // kernel for it a second time — and would be asking a question this has already answered.
    let found = Place {
        root: prompt::git_root_of(Path::new(&cwd)).map(|root| root.to_string_lossy().into_owned()),
        cwd,
    };
    *slot = Some(found.clone());
    Some(found)
}

/// This directory, then this worktree, and nothing at all when neither knows the answer.
///
/// The exact directory is asked outright every time, so a line another terminal has just learnt in
/// it is offered here immediately. The widened question is asked only when the exact one came up
/// empty, and only when its answer is not already known — see the module note for the measurements
/// that force that difference.
pub(super) fn from_store(track: &Track, place: &Place, mode: &str, line: &str) -> Option<String> {
    if let Some(here) = track.suggestion_here(&place.cwd, mode, line)
        && let Some(tail) = tail_of(&here, line)
    {
        return Some(tail);
    }
    // Only when the exact directory had nothing: you typed it at the top of the repository and you
    // are three directories down. `dir.root` was written at visit time, so there is no `git`
    // subprocess anywhere near this path.
    let root = place.root.as_deref()?;
    let found = match remembered_answer(root, mode, line) {
        Some(known) => known,
        None => {
            let asked = track.suggestion_in_workspace(root, mode, line);
            remember_answer(root, mode, line, asked.as_deref());
            asked
        }
    };
    tail_of(&found?, line)
}

/// One answer the widened query gave, and the prefix it was asked about.
#[derive(Clone, Debug)]
struct Memo {
    root: String,
    mode: String,
    asked: String,
    found: Option<String>,
}

/// The widened query's answers. See [`from_store`] for why only that one is remembered.
static WIDENED: Mutex<Vec<Memo>> = Mutex::new(Vec::new());

/// How many prefixes are remembered before the oldest is dropped.
///
/// A bound rather than a map because the list is walked, not looked up: a line is thirty
/// keystrokes and each divergence from the suggestion adds one entry, so this is several lines'
/// worth and the walk is over a handful of short strings.
const MEMOS: usize = 32;

/// What the widened query would answer for `line`, without asking it. `None` means ask.
///
/// Two facts about a prefix search make this sound. If nothing in the worktree starts with a
/// prefix of `line`, nothing starts with `line` either. And if the best line for a shorter prefix
/// still starts with `line`, it is still the best: typing more can only remove candidates from the
/// field, and the one already at the top of the order is not among the ones removed.
///
/// Which is why the *longest* remembered prefix answers rather than any matching one. That is not
/// an optimisation: a shorter prefix whose answer still fits was, by the same argument, already
/// ranked below the longer prefix's answer and passed over.
///
/// `found == line` has to go back to the store. The query never offers the line already typed, so
/// what it would answer there is something longer that this has never seen.
fn remembered_answer(root: &str, mode: &str, line: &str) -> Option<Option<String>> {
    let memos = WIDENED.lock().ok()?;
    memos
        .iter()
        .filter(|memo| memo.root == root && memo.mode == mode && line.starts_with(&memo.asked))
        .max_by_key(|memo| memo.asked.len())
        .and_then(|memo| match &memo.found {
            None => Some(None),
            Some(found) if found.starts_with(line) && found != line => Some(Some(found.clone())),
            Some(_) => None,
        })
}

fn remember_answer(root: &str, mode: &str, asked: &str, found: Option<&str>) {
    let Ok(mut memos) = WIDENED.lock() else {
        return;
    };
    if memos.len() >= MEMOS {
        memos.remove(0);
    }
    memos.push(Memo {
        root: root.to_string(),
        mode: mode.to_string(),
        asked: asked.to_string(),
        found: found.map(str::to_string),
    });
}

/// Drop every remembered answer the line just typed could change.
///
/// A line can only join, leave or reorder the candidates for a prefix of *itself*, so a prefix it
/// does not start with is untouched and its answer stands. This is the whole invalidation: the
/// store is written at the end of the command that is about to run, and nothing else in this shell
/// can change what the widened query returns.
///
/// Another terminal can, and this does not hear about it. The cost is that a line learnt elsewhere
/// may take until the next command to be offered in a worktree this shell has already asked about
/// — one line of staleness, against a query that is otherwise milliseconds on every keystroke.
pub(super) fn forget_answers_for(line: &str, language: &str) {
    if let Ok(mut memos) = WIDENED.lock() {
        memos.retain(|memo| {
            memo.mode != language
                || !line.starts_with(&memo.asked)
                // Running the very line an answer names cannot unseat it. The store counts one
                // more of it and leaves every other candidate where it was, so the row that was
                // already top of the order is still top — which matters because accepting the
                // suggestion is what this whole path exists to make easy, and it would otherwise
                // be the one act that threw the answer away.
                || memo.found.as_deref() == Some(line)
        });
    }
}

/// Drop them all: the history itself has been replaced or wiped.
pub(super) fn forget_answers() {
    if let Ok(mut memos) = WIDENED.lock() {
        memos.clear();
    }
}

/// What is left of a remembered line once what has been typed is taken off the front.
///
/// `None` for a line that adds nothing, and for a multi-line one — see the note in
/// [`super::remembered`], which is the same trap read out of the flat set instead of the store.
fn tail_of(candidate: &str, line: &str) -> Option<String> {
    let tail = candidate.strip_prefix(line)?;
    if tail.is_empty() || candidate.contains('\n') {
        return None;
    }
    Some(tail.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interactive::recall::{SERIAL, clear, remember, remembered, seed};
    use crate::track::{Run, Step, Visit};

    /// A store of its own, so these tests never touch the process-global one — that slot can only
    /// be written once and belongs to the shell, not to whichever test got there first.
    ///
    /// The memo of widened answers *is* process-global, so it is emptied here and the caller holds
    /// [`SERIAL`] for the duration.
    fn store() -> (tempfile::TempDir, Track) {
        forget_answers();
        let dir = tempfile::tempdir().expect("a temp dir");
        let track = Track::open(&dir.path().join("track.db")).expect("the database opens");
        (dir, track)
    }

    /// One command, run in `at` and finished cleanly.
    fn ran<'a>(at: &'a str, root: Option<&'a str>, argv: &'a str, mode: &'a str) -> Step<'a> {
        Step {
            ran_in: Visit { path: at, root },
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv,
                mode,
                status: Some(0),
                duration_ms: 5,
            }),
        }
    }

    fn here(cwd: &str) -> Place {
        Place {
            cwd: cwd.to_string(),
            root: None,
        }
    }

    /// The headline: the same prefix means different things in different projects, and the flat
    /// history cannot say so because it only knows which one was typed last.
    #[test]
    fn the_directory_decides_which_line_is_offered() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", None, "cargo run --example xyz", "sh"));
        track.record(&ran("/w/beta", None, "cargo run --example abc", "sh"));

        assert_eq!(
            from_store(&track, &here("/w/alpha"), "sh", "cargo run --ex"),
            Some("ample xyz".to_string())
        );
        assert_eq!(
            from_store(&track, &here("/w/beta"), "sh", "cargo run --ex"),
            Some("ample abc".to_string())
        );
    }

    /// You typed it at the top of the repository and you are three directories down. The exact
    /// directory is asked first and the worktree only when it had nothing.
    #[test]
    fn the_worktree_answers_only_when_the_exact_directory_cannot() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_dir, track) = store();
        let root = "/w/beta";
        // Twice at the top and once downstairs, so the widened query has an order to answer in
        // rather than a coin to flip.
        track.record(&ran(root, Some(root), "cargo run --example abc", "sh"));
        track.record(&ran(root, Some(root), "cargo run --example abc", "sh"));
        track.record(&ran(
            "/w/beta/crates/api",
            Some(root),
            "cargo run --example api",
            "sh",
        ));

        let deep = Place {
            cwd: "/w/beta/crates/api".to_string(),
            root: Some(root.to_string()),
        };
        assert_eq!(
            from_store(&track, &deep, "sh", "cargo run --ex"),
            Some("ample api".to_string()),
            "what was run here wins over what was run upstairs"
        );

        let elsewhere = Place {
            cwd: "/w/beta/crates/cli".to_string(),
            root: Some(root.to_string()),
        };
        assert_eq!(
            from_store(&track, &elsewhere, "sh", "cargo run --ex"),
            Some("ample abc".to_string()),
            "and the worktree answers where nothing has ever been run"
        );

        // Outside the repository there is no root to widen to, so the store says nothing at all
        // and the caller falls back to the flat walk.
        assert_eq!(
            from_store(&track, &here("/w/gamma"), "sh", "cargo run"),
            None
        );
    }

    /// A line learnt in one language is not an alternative for the same slot in the other. This
    /// was a real bug once and the store must not be the way it comes back.
    #[test]
    fn a_line_learnt_in_one_language_is_never_offered_in_the_other() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_dir, track) = store();
        let root = Some("/w/alpha");
        track.record(&ran("/w/alpha", root, "print('shell')", "sh"));
        track.record(&ran("/w/alpha", root, "print('lua')", "lua"));

        assert_eq!(
            from_store(&track, &here("/w/alpha"), "sh", "print("),
            Some("'shell')".to_string())
        );
        assert_eq!(
            from_store(&track, &here("/w/alpha"), "lua", "print("),
            Some("'lua')".to_string())
        );

        // And the widened query is keyed by language too, or the subdirectory case would be the
        // hole the bug came back through.
        let deep = Place {
            cwd: "/w/alpha/src".to_string(),
            root: Some("/w/alpha".to_string()),
        };
        track.record(&ran("/w/alpha", root, "ls -la", "sh"));
        assert_eq!(
            from_store(&track, &deep, "sh", "ls "),
            Some("-la".to_string())
        );
        assert_eq!(from_store(&track, &deep, "lua", "ls "), None);
    }

    /// The store holds the line as typed, newlines and all, and ghost text is drawn on one row.
    #[test]
    fn a_multi_line_entry_from_the_store_is_never_suggested() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_dir, track) = store();
        track.record(&ran(
            "/w/alpha",
            None,
            "for f in *\ndo\necho $f\ndone",
            "sh",
        ));
        assert_eq!(from_store(&track, &here("/w/alpha"), "sh", "for"), None);

        // A single-line entry in the same directory is still offered.
        track.record(&ran("/w/alpha", None, "format-all", "sh"));
        assert_eq!(
            from_store(&track, &here("/w/alpha"), "sh", "for"),
            Some("mat-all".to_string())
        );
    }

    /// Nothing the store can answer means the flat walk answers, which is what oslo did before it
    /// had a store and what it still does in a directory nobody has worked in.
    #[test]
    fn an_unknown_directory_falls_back_to_the_flat_history() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", None, "cargo build", "sh"));

        assert_eq!(from_store(&track, &here("/w/nowhere"), "sh", "cargo"), None);
        seed(vec![("cargo fmt".to_string(), "sh".to_string())]);
        assert_eq!(remembered("sh", "cargo"), Some(" fmt".to_string()));
        seed(Vec::new());
    }

    /// The walk up the tree is a `stat` per level and the hint runs on every keystroke, so it
    /// happens once per directory instead.
    #[test]
    fn the_worktree_is_resolved_once_per_directory() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let cwd = std::env::current_dir()
            .expect("a working directory")
            .to_string_lossy()
            .into_owned();
        let invented = Some("/not/a/worktree".to_string());

        // An answer no walk would ever produce, filed under where we are actually standing.
        *PLACE.lock().unwrap() = Some(Place {
            cwd: cwd.clone(),
            root: invented.clone(),
        });
        assert_eq!(
            place().expect("a place").root,
            invented,
            "a directory already resolved is not walked again"
        );

        // Filed under anywhere else it is not an answer to this question, so the walk happens.
        *PLACE.lock().unwrap() = Some(Place {
            cwd: "/somewhere/else".to_string(),
            root: invented.clone(),
        });
        let fresh = place().expect("a place");
        assert_eq!(fresh.cwd, cwd, "changing directory is its own invalidation");
        assert_ne!(fresh.root, invented);
        *PLACE.lock().unwrap() = None;
    }

    /// Nothing in the worktree starts with `zz`, so nothing starts with anything longer, and the
    /// millisecond query is not repeated to find that out again.
    #[test]
    fn a_prefix_that_found_nothing_answers_for_every_longer_one() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_answers();
        remember_answer("/w/beta", "sh", "zz", None);

        assert_eq!(remembered_answer("/w/beta", "sh", "zzz"), Some(None));
        assert_eq!(
            remembered_answer("/w/beta", "sh", "z"),
            None,
            "a shorter prefix is a wider question and has to be asked"
        );
        assert_eq!(
            remembered_answer("/w/alpha", "sh", "zzz"),
            None,
            "another worktree is a different question"
        );
        assert_eq!(
            remembered_answer("/w/beta", "lua", "zzz"),
            None,
            "and so is another language"
        );
        forget_answers();
    }

    /// Typing more of a suggestion only removes candidates, and the one already at the top of the
    /// order is not among the ones removed.
    #[test]
    fn an_answer_still_ahead_of_the_field_is_not_asked_for_again() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_answers();
        remember_answer("/w/beta", "sh", "c", Some("cargo run --example abc"));

        assert_eq!(
            remembered_answer("/w/beta", "sh", "cargo run"),
            Some(Some("cargo run --example abc".to_string()))
        );
        assert_eq!(
            remembered_answer("/w/beta", "sh", "cargo test"),
            None,
            "typed away from it: the field has changed and the store has to rank it again"
        );
        assert_eq!(
            remembered_answer("/w/beta", "sh", "cargo run --example abc"),
            None,
            "typed all of it: what comes next is something this has never seen"
        );
        forget_answers();
    }

    /// The longest remembered prefix answers, and that is correctness rather than economy: an
    /// answer a longer prefix passed over was already ranked below the one it chose.
    #[test]
    fn the_prefix_that_ranked_the_narrowest_field_is_the_one_that_answers() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_answers();
        remember_answer("/w/beta", "sh", "c", Some("cargo build --release"));
        remember_answer("/w/beta", "sh", "cargo b", Some("cargo build --workspace"));

        assert_eq!(
            remembered_answer("/w/beta", "sh", "cargo bu"),
            Some(Some("cargo build --workspace".to_string())),
            "both still fit, and the one asked of the narrower field wins"
        );
        forget_answers();
    }

    /// The line just typed is about to become a row in the store, so every answer it could change
    /// is dropped — and no others, or the memo would be worth nothing.
    #[test]
    fn a_line_just_typed_forgets_the_answers_it_could_change() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        forget_answers();
        remember_answer("/w/beta", "sh", "car", Some("cargo build"));
        remember_answer("/w/beta", "sh", "ls", None);
        remember_answer("/w/beta", "lua", "car", Some("caret()"));

        remember("cargo run --example new", "sh");

        assert_eq!(
            remembered_answer("/w/beta", "sh", "cargo r"),
            None,
            "the new line could be the answer to that now"
        );
        assert_eq!(
            remembered_answer("/w/beta", "sh", "ls -la"),
            Some(None),
            "a prefix the new line does not start with cannot have changed"
        );
        assert_eq!(
            remembered_answer("/w/beta", "lua", "care"),
            Some(Some("caret()".to_string())),
            "and a shell line changes nothing about what Lua would be offered"
        );

        // Accepting a suggestion is the one thing that must not throw it away.
        remember_answer("/w/beta", "sh", "car", Some("cargo build"));
        remember("cargo build", "sh");
        assert_eq!(
            remembered_answer("/w/beta", "sh", "cargo b"),
            Some(Some("cargo build".to_string())),
            "one more run of the line already at the top of the order leaves it there"
        );

        // `history -c` and a language switch both replace the set outright, and neither may leave
        // an answer behind that came from the lines being forgotten.
        clear();
        assert_eq!(remembered_answer("/w/beta", "sh", "ls -la"), None);
        seed(Vec::new());
    }
}
