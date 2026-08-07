//! What a directory changed, so that leaving it can change it back.
//!
//! Used for two kinds of shell state now — environment variables and aliases — because the shape of
//! the problem is identical for both: record `name -> (before, after)`, and undoing is
//! [`Diff::reverse`]. Anything else a `.env.lua` can set reversibly should come through here too
//! rather than growing its own bookkeeping.
//!
//! **Unloading is the hard half of a directory environment, and it is the half that hand-rolled
//! versions get wrong.** Setting variables on arrival is easy; the thing that makes the feature
//! trustworthy is that walking out puts the shell back exactly as it was — including variables the
//! rc file *unset*, and variables that had no value before and must end up with no value again
//! rather than with an empty one.
//!
//! direnv records this as a `{prev, next}` pair of whole environments and serialises it into
//! `DIRENV_DIFF` because it is a separate process with nowhere else to keep state
//! (`internal/cmd/env_diff.go`). oslo is the shell, so the same pair lives in memory, and unloading
//! is [`Diff::reverse`] — direnv's own trick of simply swapping the two halves.
//!
//! Only the keys that actually differ are kept. A whole-environment snapshot per directory would
//! restore variables the rc file never touched, which would quietly undo anything you changed by
//! hand while you were standing there.

use std::collections::BTreeMap;

/// What happened to one name.
///
/// The three marks direnv uses, and for the same reason: `+FOO ~PATH -BAR` says in eleven
/// characters what a sentence would take a line to say, and the shape is already in people's eyes
/// from `git status` and from direnv itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// Something you had is gone. Rare, and the most surprising thing a directory can do.
    Removed,
    /// Something you had is different — `~PATH` on nearly every dev shell.
    Modified,
    /// Something new. Expected, numerous, and the first thing to drop when the line is full.
    Added,
}

impl Change {
    pub fn mark(self) -> char {
        match self {
            Change::Added => '+',
            Change::Modified => '~',
            Change::Removed => '-',
        }
    }
}

/// What changed, as `name -> (before, after)`.
///
/// Sorted, so that reporting it is stable and two diffs of the same change compare equal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff<V = String> {
    /// `name -> (before, after)`, where `None` on either side means the variable did not exist.
    ///
    /// That distinction is load-bearing: restoring `FOO` to `Some("")` leaves an empty variable,
    /// which `[ -n "$FOO" ]` reads differently from the `None` that was there before.
    changes: BTreeMap<String, (Option<V>, Option<V>)>,
}

impl<V: Clone + PartialEq> Diff<V> {
    /// The difference between two snapshots, keeping only what moved.
    pub fn between(before: &BTreeMap<String, V>, after: &BTreeMap<String, V>) -> Diff<V> {
        let mut changes = BTreeMap::new();
        for (name, old) in before {
            match after.get(name) {
                Some(new) if new == old => {}
                Some(new) => {
                    changes.insert(name.clone(), (Some(old.clone()), Some(new.clone())));
                }
                None => {
                    changes.insert(name.clone(), (Some(old.clone()), None));
                }
            }
        }
        for (name, new) in after {
            if !before.contains_key(name) {
                changes.insert(name.clone(), (None, Some(new.clone())));
            }
        }
        Diff { changes }
    }

    /// The same change, backwards. Applying this undoes it.
    pub fn reverse(&self) -> Diff<V> {
        Diff {
            changes: self
                .changes
                .iter()
                .map(|(name, (before, after))| (name.clone(), (after.clone(), before.clone())))
                .collect(),
        }
    }

    /// Whether anything moved at all.
    ///
    /// Nothing calls this today; it is here because `len` without `is_empty` is a clippy denial,
    /// and the lint is right — a caller reaching for one will reach for the other.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// How many variables this touches.
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// The names it touches, for `direnv status`.
    pub fn names(&self) -> Vec<&str> {
        self.changes.keys().map(String::as_str).collect()
    }

    /// What happened to each name, for reporting a load.
    ///
    /// Sorted **most surprising first**: removals, then modifications, then additions, and
    /// alphabetically within each.
    ///
    /// The ordering is the difference between a useful line and a useless one. A Nix dev shell adds
    /// thirty boilerplate variables and takes away one that mattered; sorted the other way the line
    /// fills with `+AR_FOR_BUILD +AS_FOR_BUILD …` and the `-GITHUB_TOKEN` you needed to see is in
    /// the part that got truncated.
    pub fn changes(&self) -> Vec<(&str, Change)> {
        let mut out: Vec<(&str, Change)> = self
            .changes
            .iter()
            .map(|(name, (before, after))| {
                let kind = match (before.is_some(), after.is_some()) {
                    (false, true) => Change::Added,
                    (true, false) => Change::Removed,
                    _ => Change::Modified,
                };
                (name.as_str(), kind)
            })
            .collect();
        out.sort_by_key(|(name, kind)| (*kind, *name));
        out
    }

    /// Every change as `(name, value_to_apply)`, where `None` means remove it.
    pub fn to_apply(&self) -> Vec<(&str, Option<&V>)> {
        self.changes
            .iter()
            .map(|(name, (_, after))| (name.as_str(), after.as_ref()))
            .collect()
    }

    /// A diff built from what it should *apply*, which is the inverse of [`Diff::to_apply`].
    ///
    /// For a record that arrived from somewhere other than a comparison — one carried in the
    /// environment from a parent shell, which is a diff that has already been reversed and has no
    /// "before" side left to speak of. Only [`Diff::to_apply`] is meaningful on the result; asking
    /// it what *changed* would describe the undo rather than the load.
    pub fn to_restore(pairs: Vec<(String, Option<V>)>) -> Diff<V> {
        Diff {
            changes: pairs
                .into_iter()
                .map(|(name, after)| (name, (None, after)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `to_apply` hands back borrowed values; these read better compared as owned strings.
    fn applied(diff: &Diff<String>) -> Vec<(&str, Option<&str>)> {
        diff.to_apply()
            .into_iter()
            .map(|(name, value)| (name, value.map(String::as_str)))
            .collect()
    }

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// A variable that did not exist must go back to not existing, not to empty.
    ///
    /// The difference is visible to every script that uses `${FOO:-default}` or `[ -n "$FOO" ]`,
    /// so restoring the wrong one is a silent behaviour change in code that has nothing to do with
    /// this feature.
    #[test]
    fn a_variable_that_was_absent_is_restored_to_absent() {
        let before = env(&[]);
        let after = env(&[("DATABASE_URL", "postgres://local")]);
        let diff = Diff::between(&before, &after);

        assert_eq!(
            applied(&diff),
            vec![("DATABASE_URL", Some("postgres://local"))]
        );
        assert_eq!(
            applied(&diff.reverse()),
            vec![("DATABASE_URL", None)],
            "leaving must remove it, not blank it"
        );
    }

    /// A variable the rc file unset must come back with its old value.
    #[test]
    fn an_unset_variable_comes_back() {
        let before = env(&[("EDITOR", "vim")]);
        let after = env(&[]);
        let diff = Diff::between(&before, &after);

        assert_eq!(applied(&diff), vec![("EDITOR", None)]);
        assert_eq!(applied(&diff.reverse()), vec![("EDITOR", Some("vim"))]);
    }

    /// Only what moved is recorded, so anything changed by hand while standing there survives.
    #[test]
    fn untouched_variables_are_not_in_the_diff() {
        let before = env(&[("HOME", "/home/u"), ("PATH", "/bin")]);
        let after = env(&[("HOME", "/home/u"), ("PATH", "/bin"), ("NEW", "1")]);
        let diff = Diff::between(&before, &after);

        assert_eq!(diff.names(), vec!["NEW"]);
        assert_eq!(
            diff.len(),
            1,
            "restoring HOME and PATH was never this diff's business"
        );
    }

    /// Reversing twice is the original, which is what makes unload-then-reload safe.
    #[test]
    fn reversing_twice_is_where_you_started() {
        let diff = Diff::between(
            &env(&[("A", "1"), ("B", "2")]),
            &env(&[("A", "9"), ("C", "3")]),
        );
        assert_eq!(diff.reverse().reverse(), diff);
    }

    /// The three marks, and the order that keeps the surprising names at the front.
    #[test]
    fn changes_are_marked_and_ordered_additions_first() {
        let diff = Diff::between(
            &env(&[("GONE", "1"), ("SAME", "s"), ("MOVED", "old")]),
            &env(&[("SAME", "s"), ("MOVED", "new"), ("FRESH", "2")]),
        );
        assert_eq!(
            diff.changes(),
            vec![
                ("GONE", Change::Removed),
                ("MOVED", Change::Modified),
                ("FRESH", Change::Added),
            ],
            "most surprising first, so truncation drops the boilerplate"
        );
        assert_eq!(Change::Added.mark(), '+');
        assert_eq!(Change::Modified.mark(), '~');
        assert_eq!(Change::Removed.mark(), '-');
    }

    #[test]
    fn an_unchanged_environment_has_nothing_to_undo() {
        let same = env(&[("A", "1")]);
        assert_eq!(Diff::between(&same, &same).len(), 0);
    }
}
