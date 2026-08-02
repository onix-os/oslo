//! Where `cd` looks after the directory it was handed turned out not to exist.
//!
//! This is the whole of `cd`'s cleverness, and it is deliberately in its own file behind one entry
//! point, because everything about it is conditional on a failure that has already happened. POSIX
//! has been satisfied before a line of it runs: the operand was resolved against `$PWD`, `CDPATH`
//! was searched, and the kernel refused. Only then is there a question left to ask.
//!
//! It cannot run at all without a store, and only the interactive loop installs one. So `cd
//! nonexistent` in a script, in `oslo -c`, or in a subshell reaches [`jump`], finds no store, and
//! gets the same diagnostic and the same status 1 it always got — not because a flag was checked
//! but because there is nothing to consult. That is `expand::sugar`'s precedent and `autocd`'s
//! principle: a script's meaning must not depend on which directories happen to exist beside it.

use super::chdir::{PathMode, attempt_directory, logical_pwd};
use crate::env::scope::Environment;
use crate::interactive::prompt::git_root;
use crate::track;
use crate::track::Track;
use crate::track::score::{Query, Ranked, rank};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The word that means "the top of this working tree" rather than a directory of that name.
///
/// A real `./root` still wins, because the plain move is tried first and only its failure gets
/// here. That precedence is what saves a machine with an actual `root` directory in it.
const REPOSITORY: &str = "root";

/// How many remembered directories the ranker is offered.
///
/// The database orders by the same frecency expression the ranker does, so this is a cut through an
/// already-sorted list rather than an arbitrary sample — but it is generous, because match quality
/// outranks frecency and a better-matching row can be well down the score order.
const CANDIDATES: usize = 200;

/// Where `cd {operand}` should go now that `{operand}` is not a directory, or `None`.
///
/// On success the shell has already moved: the destination is validated by a real `chdir` through
/// [`attempt_directory`], so `$PWD` and `$OLDPWD` are written by the one function that owns them
/// and the requested `-L`/`-P` reading is honoured for the jump exactly as for any other `cd`.
pub fn jump(env: &mut Environment, operand: &str, mode: PathMode) -> Option<String> {
    // No store, no cleverness. This is the gate, and it is structural rather than remembered.
    let track = track::store()?;

    // `cd root` is the top of the working tree, walked for without a `git` subprocess. Outside a
    // repository the word means nothing in particular, so it falls through to being a name like
    // any other — somebody who really does have a `root` directory they visit should reach it.
    if operand == REPOSITORY
        && let Some(top) = git_root()
        && let Some(path) = top.to_str()
        && let Ok(destination) = attempt_directory(env, path, mode)
    {
        return Some(destination);
    }

    let here = logical_pwd(env);
    let workspace = git_root().map(|top| top.to_string_lossy().into_owned());
    let hit = destination(track, operand, &here, workspace.as_deref())?;
    attempt_directory(env, &hit, mode).ok()
}

/// The remembered directory `operand` names, from where the shell is standing.
///
/// Naming a directory is an indexed range on the folded final component; the tiers no index can
/// serve are a scan, and are only paid for when naming it found nothing — and only ever for a `cd`
/// a person typed, never per keystroke. Both exclude `here` in SQL rather than by a comparison made
/// afterwards, which is the one thing zoxide gets structurally wrong: it excludes `$PWD` from its
/// *shell function* by string equality against `pwd`, and so silently fails whenever `pwd -L` and
/// `pwd -P` disagree.
fn destination(
    track: &Track,
    operand: &str,
    here: &str,
    workspace: Option<&str>,
) -> Option<String> {
    let query = Query::one(operand);
    if query.is_empty() {
        return None;
    }
    let now = epoch_seconds();
    let named = track.directories_named(operand, here, CANDIDATES);
    pick(rank(named, &query, now, workspace)).or_else(|| {
        let every = track.directories_ranked(here, CANDIDATES);
        pick(rank(every, &query, now, workspace))
    })
}

/// The best-ranked candidate that is still a directory on disk.
///
/// A remembered path that has since been deleted or unmounted is skipped rather than jumped to and
/// rather than dropped from the store on sight: an unplugged drive is not a directory that stopped
/// existing, and deciding that is the prune sweep's job, on its own schedule.
fn pick(ranked: Vec<Ranked>) -> Option<String> {
    ranked
        .into_iter()
        .find(|hit| Path::new(&hit.path).is_dir())
        .map(|hit| hit.path)
}

/// Now, as `dir.last_visit` counts it.
fn epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{Step, Visit};
    use std::fs;
    use std::path::PathBuf;

    /// A store, and a scratch tree to remember directories in.
    ///
    /// The tree is under `target/` rather than under `$TMPDIR`, because the store deliberately
    /// refuses to remember anything below `/tmp` — a build directory is not somewhere anybody wants
    /// to be sent back to — and a test that recorded nothing would pass for the wrong reason.
    fn scratch() -> (tempfile::TempDir, PathBuf, Track) {
        let target = concat!(env!("CARGO_MANIFEST_DIR"), "/target");
        fs::create_dir_all(target).expect("a target directory");
        let dir = tempfile::Builder::new()
            .prefix("oslo-jump")
            .tempdir_in(target)
            .expect("a scratch directory");
        let base = dir.path().canonicalize().expect("canonicalize");
        let track = Track::open(&base.join("track.db")).expect("the database opens");
        (dir, base, track)
    }

    /// Walk into `path` `visits` times, creating it.
    fn visit(track: &Track, base: &Path, path: &str, visits: usize) -> String {
        let full = base.join(path);
        fs::create_dir_all(&full).expect("a directory to visit");
        let full = full.to_string_lossy().into_owned();
        for _ in 0..visits {
            assert!(track.record(&Step {
                ran_in: Visit::at(base.to_str().expect("utf-8")),
                moved_to: Some(Visit::at(&full)),
                dwell_ms: 0,
                run: None,
            }));
        }
        full
    }

    /// The feature in one assertion: a name that is not a directory here is a directory you have
    /// been to before.
    #[test]
    fn a_name_that_is_not_a_directory_finds_the_one_you_meant() {
        let (_dir, base, track) = scratch();
        let project = visit(&track, &base, "work/project", 3);
        visit(&track, &base, "other/place", 9);

        let here = base.to_str().expect("utf-8");
        assert_eq!(
            destination(&track, "project", here, None),
            Some(project.clone())
        );
        // Reached by a weaker tier too, which is the scan rather than the index — but only because
        // it has been visited more than once.
        assert_eq!(destination(&track, "roje", here, None), Some(project));
        assert_eq!(
            destination(&track, "nowhere", here, None),
            None,
            "a name nothing answers to is still an error"
        );
        assert_eq!(destination(&track, "", here, None), None);
    }

    /// Naming the directory you are already in must not answer "you are already there" — there is
    /// another directory of that name and it is the one you meant.
    #[test]
    fn where_you_are_standing_is_never_the_answer() {
        let (_dir, base, track) = scratch();
        let mine = visit(&track, &base, "alpha/src", 20);
        let theirs = visit(&track, &base, "beta/src", 2);

        assert_eq!(
            destination(&track, "src", &mine, None),
            Some(theirs.clone()),
            "standing in one `src`, `cd src` is the other one"
        );
        assert_eq!(
            destination(&track, "src", &theirs, None),
            Some(mine),
            "and the other way round"
        );
    }

    /// The project you are standing in wins a tie, which is zoxide #929.
    #[test]
    fn the_directory_in_this_workspace_wins() {
        let (_dir, base, track) = scratch();
        let near = visit(&track, &base, "alpha/notes", 2);
        let far = visit(&track, &base, "beta/notes", 40);

        let here = base.join("alpha").to_string_lossy().into_owned();
        assert_eq!(
            destination(&track, "notes", &here, Some(&here)),
            Some(near),
            "the one in this worktree, however much the other was used"
        );
        assert_eq!(
            destination(&track, "notes", &here, None),
            Some(far),
            "with no worktree to appeal to, frequency decides"
        );
    }

    /// A directory that has since been deleted, or is on a drive that is not plugged in, is passed
    /// over rather than jumped to — and the shell falls to the next best answer instead of failing.
    #[test]
    fn a_directory_that_is_no_longer_there_is_passed_over() {
        let (_dir, base, track) = scratch();
        let gone = visit(&track, &base, "gone/notes", 50);
        let still_here = visit(&track, &base, "kept/notes", 2);
        fs::remove_dir_all(&gone).expect("delete it behind the store's back");

        let here = base.to_str().expect("utf-8");
        assert_eq!(destination(&track, "notes", here, None), Some(still_here));
    }

    /// One stray `cd` into a typo must not become a permanent destination.
    #[test]
    fn a_directory_seen_once_is_reachable_only_by_its_exact_name() {
        let (_dir, base, track) = scratch();
        let typo = visit(&track, &base, "prjects", 1);

        let here = base.to_str().expect("utf-8");
        assert_eq!(destination(&track, "prj", here, None), None);
        assert_eq!(destination(&track, "prjects", here, None), Some(typo));
    }
}
