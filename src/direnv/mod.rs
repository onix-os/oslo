//! Directory environments: one `.env.lua` per project.
//!
//! `direnv`, built in. See `docs/research/direnv.md` for what was read and what was deliberately not
//! copied; the short version is that most of direnv's machinery exists because it is an external
//! binary talking to a shell it did not write, and oslo *is* the shell. There is no bash subprocess,
//! no `DIRENV_DIFF` serialised into your environment, no prompt hook, and no `eval` protocol. The
//! state lives in memory and the load happens on the `cd` path.
//!
//! What is copied exactly is the part that is not incidental: **the allow gate**. A file in a
//! directory getting to run code when you walk in is arbitrary code execution by anyone who can get
//! you to clone a repository, and [`allow`] is the answer, hash for hash.
//!
//! # The lifecycle
//!
//! On a directory change, and only on a directory change:
//!
//! 1. find the nearest ancestor with rc files;
//! 2. if it is the same set, unchanged, do nothing — the common case, and it costs one `stat` each;
//! 3. otherwise **unload first**, always, so moving between two projects cannot leave the first
//!    one's variables behind;
//! 4. then load the new one, if it is allowed.

pub mod allow;
pub mod diff;
pub mod find;
mod handle;

pub use handle::{install, installed, request_reload, take_reload_request, with};

use crate::env::Environment;
use allow::{Allow, Status};
use diff::Diff;
use find::Rc;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

/// What is loaded right now, if anything.
struct Loaded {
    /// The directory the rc files live in.
    owner: PathBuf,
    /// The file as it was when it was read, so an edit reloads it.
    watch: (PathBuf, Stamp),
    /// What to undo on the way out.
    undo: Diff,
}

/// The shell's directory-environment state.
pub struct Direnv {
    allow: Allow,
    loaded: Option<Loaded>,
    /// Set when a decision has made the loaded state wrong, so the next [`Direnv::arrive`] does the
    /// work again even though the directory has not changed.
    stale: bool,
    /// Paths already reported as needing `direnv allow`, so the notice is printed once rather than
    /// on every prompt. direnv prints on every hook fire; that is noise, and noise gets ignored,
    /// which is the last thing a security prompt should be.
    told: Vec<PathBuf>,
}

/// What happened, for the caller to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Nothing applies and nothing was loaded.
    Idle,
    /// Loaded `owner`, touching this many variables.
    Loaded { owner: PathBuf, vars: usize },
    /// Left a directory environment behind.
    Unloaded { owner: PathBuf },
    /// Found something, but it has not been allowed. Nothing was read.
    Blocked { path: PathBuf },
    /// Found something explicitly refused.
    Denied { path: PathBuf },
    /// An allowed file was read and something in it went wrong.
    ///
    /// Reported rather than swallowed, and *beside* a `Loaded` rather than instead of it: whatever
    /// ran before the failure has taken effect, and pretending otherwise would leave the shell in a
    /// state it says it is not in.
    Failed { path: PathBuf, problem: String },
}

impl Direnv {
    pub fn new(xdg_data: Option<&str>, home: Option<&str>) -> Direnv {
        Direnv {
            allow: Allow::new(xdg_data, home),
            loaded: None,
            stale: false,
            told: Vec::new(),
        }
    }

    /// The allow store, for the `direnv` builtin.
    pub fn permissions(&self) -> &Allow {
        &self.allow
    }

    /// The directory whose environment is loaded, if any.
    pub fn active(&self) -> Option<&Path> {
        self.loaded.as_ref().map(|l| l.owner.as_path())
    }

    /// The variables the loaded environment is holding.
    pub fn holding(&self) -> Vec<&str> {
        self.loaded
            .as_ref()
            .map(|l| l.undo.names())
            .unwrap_or_default()
    }

    /// Forget that a path was reported, so the notice prints again after `direnv deny` or an edit.
    pub fn tell_again(&mut self, path: &Path) {
        self.told.retain(|told| told != path);
    }

    /// Mark the loaded state as no longer reflecting the decisions on record.
    ///
    /// **Not `loaded = None`.** Dropping the record is what an early version did, and it leaks: the
    /// undo diff is the only thing that knows how to take the variables back out, so forgetting it
    /// after `direnv deny` would leave a denied environment applied with no way to remove it. The
    /// record is kept and merely marked stale, so the next arrival unloads properly first.
    pub fn invalidate(&mut self) {
        self.stale = true;
        self.told.clear();
    }

    /// Bring the environment into line with `dir`. The whole feature, from the caller's side.
    ///
    /// `run` evaluates the `.env.lua` and reports what went wrong. A callback rather than something
    /// this module does, because running Lua needs the engine and that belongs to the read loop.
    ///
    /// **The environment lock is not held while it runs, and must not be.** The file's whole job is
    /// to call `oslo.set_var` and friends, and those take the same lock with `try_lock` — holding it
    /// here made every one of them fail with "shell state is busy" while the load reported success.
    /// So the lock is taken twice, briefly, for the before and after snapshots, and released around
    /// the call in between.
    pub fn arrive(
        &mut self,
        env: &Mutex<Environment>,
        dir: &Path,
        run: &mut dyn FnMut(&Rc) -> Result<(), String>,
    ) -> Vec<Event> {
        let rc = find::applicable(dir);
        let owner = rc.as_ref().and_then(find::owner);

        // Standing in the same environment, with nothing edited: the overwhelmingly common case.
        if !self.stale
            && let Some(loaded) = &self.loaded
            && owner.as_deref() == Some(loaded.owner.as_path())
            && !self.changed_underneath()
        {
            return Vec::new();
        }
        self.stale = false;

        let mut events = Vec::new();
        // Unload before load, always. Moving from one project to another must not carry anything
        // across, and doing this in the other order silently merges the two.
        if let Some(event) = self.unload(env) {
            events.push(event);
        }
        let Some(rc) = rc else {
            return events;
        };

        events.extend(self.load(env, &rc, run));
        events
    }

    /// Whether any loaded file has been written since it was read.
    fn changed_underneath(&self) -> bool {
        let Some(loaded) = &self.loaded else {
            return false;
        };
        let (path, when) = &loaded.watch;
        stamp(path) != *when
    }

    /// Put back everything the loaded environment changed.
    fn unload(&mut self, env: &Mutex<Environment>) -> Option<Event> {
        let loaded = self.loaded.take()?;
        let mut guard = lock(env)?;
        for (name, value) in loaded.undo.to_apply() {
            match value {
                Some(value) => {
                    guard.set_var(name, value, true);
                }
                None => guard.unset_var(name),
            }
        }
        drop(guard);
        Some(Event::Unloaded {
            owner: loaded.owner,
        })
    }

    /// Check the gate, then read.
    fn load(
        &mut self,
        env: &Mutex<Environment>,
        rc: &Rc,
        run: &mut dyn FnMut(&Rc) -> Result<(), String>,
    ) -> Vec<Event> {
        match self.allow.status(&rc.path) {
            Status::Denied => {
                return vec![Event::Denied {
                    path: rc.path.clone(),
                }];
            }
            Status::NotAllowed => {
                // Reported once per path. A warning printed on every prompt is a warning nobody
                // reads, which is the worst outcome for a warning about running someone's code.
                if self.told.contains(&rc.path) {
                    return Vec::new();
                }
                self.told.push(rc.path.clone());
                return vec![Event::Blocked {
                    path: rc.path.clone(),
                }];
            }
            Status::Allowed => {}
        }

        let Some(before) = lock(env).map(|guard| snapshot(&guard)) else {
            return Vec::new();
        };
        let watch = (rc.path.clone(), stamp(&rc.path));
        // No lock held here. See the note on `arrive`.
        let outcome = run(rc);
        let Some(after) = lock(env).map(|guard| snapshot(&guard)) else {
            return Vec::new();
        };
        // **A file that failed half way still had its first half take effect**, so the diff is
        // taken regardless and the failure reported beside it. Discarding the load here would
        // leave those variables set with nothing recorded to unset them.
        let undo = Diff::between(&before, &after).reverse();

        let Some(owner) = find::owner(rc) else {
            return Vec::new();
        };
        let vars = undo.len();
        self.loaded = Some(Loaded {
            owner: owner.clone(),
            watch,
            undo,
        });
        let mut events = vec![Event::Loaded { owner, vars }];
        if let Err(problem) = outcome {
            events.push(Event::Failed {
                path: rc.path.clone(),
                problem,
            });
        }
        events
    }
}

/// Every exported variable, which is what a directory environment is about.
fn snapshot(env: &Environment) -> BTreeMap<String, String> {
    env.exported_vars().into_iter().collect()
}

/// The environment, or `None` if another holder has poisoned or is holding the lock.
///
/// A directory environment that cannot reach the shell state does nothing at all, which is the only
/// safe direction: half-applying it would leave variables set with no record of how to remove them.
fn lock(env: &Mutex<Environment>) -> Option<MutexGuard<'_, Environment>> {
    env.lock().ok()
}

/// What a file looked like when it was read: when it changed, and how long it was.
///
/// **Not the mtime alone.** Its granularity is one second on some filesystems, and editors that
/// write through a temporary file can even preserve it — so an rc file edited and re-entered
/// quickly would be reloaded without being re-checked against the allow list, which is the one
/// failure this whole module exists to prevent. The length is free from the same `stat` and closes
/// the common case. A file edited to the same length within the mtime granularity still slips
/// through, and the hash check on the next genuine reload is what catches that.
type Stamp = (Option<SystemTime>, Option<u64>);

fn stamp(path: &Path) -> Stamp {
    match std::fs::metadata(path) {
        Ok(meta) => (meta.modified().ok(), Some(meta.len())),
        Err(_) => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every test here must use variable names no other test uses.**
    ///
    /// `set_var(.., export: true)` calls `environ_set`, which writes the *process* environment so
    /// that children inherit it. libtest runs these in parallel and `Environment::new()` snapshots
    /// that process environment, so one test exporting `A` puts `A` in another test's *before*
    /// snapshot — the diff then records no change, unload has nothing to undo, and the failure
    /// looks like a bug in this module rather than crosstalk between tests. It cost an afternoon
    /// once already; the `OSLO_T_` prefix is the cheap fix.
    fn shell() -> Mutex<Environment> {
        Mutex::new(Environment::new())
    }

    /// Read a variable the way a caller outside the lock would.
    fn var(env: &Mutex<Environment>, name: &str) -> Option<String> {
        env.lock().unwrap().get_var(name).map(str::to_string)
    }

    /// A stand-in for the Lua engine, which the library cannot reach from here.
    ///
    /// Reads the file as `NAME=VALUE` lines and exports them. These tests are about the lifecycle —
    /// what loads, what unloads, what the allow gate refuses — and none of that depends on which
    /// language did the setting. The real evaluator is exercised through the pty harness.
    /// Built per test so it can reach the same environment the loader is diffing, taking the lock
    /// itself — which is exactly what a real `.env.lua` does through `oslo.set_var`.
    fn pairs_into(env: &Mutex<Environment>) -> impl FnMut(&Rc) -> Result<(), String> + '_ {
        move |rc: &Rc| {
            let source = std::fs::read_to_string(&rc.path).map_err(|e| e.to_string())?;
            let mut guard = env.lock().map_err(|_| "locked".to_string())?;
            for line in source.lines() {
                if let Some((name, value)) = line.trim().split_once('=') {
                    guard.set_var(name.trim(), value.trim(), true);
                }
            }
            Ok(())
        }
    }

    fn rc_in(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).expect("write");
        path
    }

    /// Nothing is read until it is allowed, and the notice is printed once.
    #[test]
    fn an_unallowed_file_is_not_read_and_is_reported_once() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        rc_in(project.path(), find::NAME, "SECRET=leaked\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        let env = shell();

        let events = direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert!(matches!(events.as_slice(), [Event::Blocked { .. }]));
        assert_eq!(
            var(&env, "SECRET").as_deref(),
            None,
            "refused means not read"
        );

        // The second arrival says nothing: a warning shown on every prompt is a warning nobody
        // reads, which is the worst outcome for this particular warning.
        direnv.loaded = None;
        assert!(
            direnv
                .arrive(&env, project.path(), &mut pairs_into(&env))
                .is_empty()
        );
    }

    /// The whole point: arriving sets, leaving restores.
    #[test]
    fn arriving_loads_and_leaving_puts_everything_back() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let path = rc_in(
            project.path(),
            find::NAME,
            "DATABASE_URL=postgres://local\n",
        );

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let env = shell();
        env.lock().unwrap().set_var("EDITOR", "vim", true);

        direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert_eq!(
            var(&env, "DATABASE_URL").as_deref(),
            Some("postgres://local")
        );

        direnv.arrive(&env, elsewhere.path(), &mut pairs_into(&env));
        assert_eq!(
            var(&env, "DATABASE_URL").as_deref(),
            None,
            "leaving must remove it, not blank it"
        );
        assert_eq!(
            var(&env, "EDITOR").as_deref(),
            Some("vim"),
            "and touch nothing else"
        );
    }

    /// Moving straight from one project to another must not merge them.
    #[test]
    fn one_project_never_leaks_into_the_next() {
        let store = tempfile::tempdir().expect("temp dir");
        let first = tempfile::tempdir().expect("temp dir");
        let second = tempfile::tempdir().expect("temp dir");
        let one = rc_in(first.path(), find::NAME, "ONLY_IN_FIRST=1\n");
        let two = rc_in(second.path(), find::NAME, "ONLY_IN_SECOND=2\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&one).expect("allow");
        direnv.permissions().allow(&two).expect("allow");
        let env = shell();

        direnv.arrive(&env, first.path(), &mut pairs_into(&env));
        direnv.arrive(&env, second.path(), &mut pairs_into(&env));

        assert_eq!(var(&env, "ONLY_IN_SECOND").as_deref(), Some("2"));
        assert_eq!(
            var(&env, "ONLY_IN_FIRST").as_deref(),
            None,
            "unload has to happen before load, or the two environments merge"
        );
    }

    /// Standing still costs nothing, which is what makes this affordable on every `cd`.
    #[test]
    fn staying_put_does_no_work() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), find::NAME, "OSLO_T_STAY=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let env = shell();

        assert!(
            !direnv
                .arrive(&env, project.path(), &mut pairs_into(&env))
                .is_empty()
        );
        assert!(
            direnv
                .arrive(&env, project.path(), &mut pairs_into(&env))
                .is_empty(),
            "a second arrival in the same place is not a reload"
        );
    }

    /// Denying a *loaded* environment must take its variables back out.
    ///
    /// The bug this pins: an early version dropped the loaded record on a decision, and the undo
    /// diff is the only thing that knows how to remove the variables — so a denied environment
    /// stayed applied with nothing able to unload it. Marking the record stale instead means the
    /// next arrival unloads properly first.
    #[test]
    fn denying_what_is_loaded_unloads_it() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), find::NAME, "OSLO_T_DENY=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let env = shell();
        direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert_eq!(var(&env, "OSLO_T_DENY").as_deref(), Some("1"));

        direnv.permissions().deny(&path).expect("deny");
        direnv.invalidate();

        // Standing in the very same directory, which the early-return would otherwise skip.
        let events = direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert!(
            events.iter().any(|e| matches!(e, Event::Unloaded { .. })),
            "the record has to survive long enough to be undone: {events:?}"
        );
        assert_eq!(
            var(&env, "OSLO_T_DENY").as_deref(),
            None,
            "a denied environment must not stay applied"
        );
    }

    /// Allowing takes effect where you are standing, not on the next `cd`.
    #[test]
    fn allowing_loads_without_moving() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), find::NAME, "OSLO_T_NOW=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        let env = shell();
        direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert_eq!(
            var(&env, "OSLO_T_NOW").as_deref(),
            None,
            "blocked until allowed"
        );

        direnv.permissions().allow(&path).expect("allow");
        direnv.invalidate();
        direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert_eq!(
            var(&env, "OSLO_T_NOW").as_deref(),
            Some("1"),
            "`direnv allow` has to work where you already are"
        );
    }

    /// A subdirectory of the project is still the project.
    #[test]
    fn walking_deeper_stays_loaded() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let deep = project.path().join("src/inner");
        std::fs::create_dir_all(&deep).expect("mkdir");
        let path = rc_in(project.path(), find::NAME, "OSLO_T_DEEP=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let env = shell();

        direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert!(direnv.arrive(&env, &deep, &mut pairs_into(&env)).is_empty());
        assert_eq!(var(&env, "OSLO_T_DEEP").as_deref(), Some("1"));
    }

    /// Editing an allowed file revokes it, so the next arrival must refuse rather than reload.
    #[test]
    fn an_edit_revokes_and_the_environment_comes_back_out() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), find::NAME, "OSLO_T_EDIT_A=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let env = shell();
        direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert_eq!(var(&env, "OSLO_T_EDIT_A").as_deref(), Some("1"));

        // Rewrite it. The mtime moves, so the next arrival re-checks, and the hash no longer matches.
        std::thread::sleep(std::time::Duration::from_millis(10));
        rc_in(
            project.path(),
            find::NAME,
            "OSLO_T_EDIT_A=1\nOSLO_T_EDIT_B=2\n",
        );

        let events = direnv.arrive(&env, project.path(), &mut pairs_into(&env));
        assert!(
            events.iter().any(|e| matches!(e, Event::Blocked { .. })),
            "an edited file has to be allowed again: {events:?}"
        );
        assert_eq!(
            var(&env, "OSLO_T_EDIT_A").as_deref(),
            None,
            "and the old values come back out"
        );
        assert_eq!(
            var(&env, "OSLO_T_EDIT_B").as_deref(),
            None,
            "the new ones never went in"
        );
    }
}
