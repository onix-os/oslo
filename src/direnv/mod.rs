//! Directory environments: `.envrc`, `.env` and `.env.lua`.
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
pub mod dotenv;
pub mod find;

use crate::env::Environment;
use allow::{Allow, Status};
use diff::Diff;
use find::{Kind, Rc};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What is loaded right now, if anything.
struct Loaded {
    /// The directory the rc files live in.
    owner: PathBuf,
    /// Every file that was loaded, with the stamp it had — so an edit reloads it.
    watches: Vec<(PathBuf, Stamp)>,
    /// What to undo on the way out.
    undo: Diff,
}

/// The shell's directory-environment state.
pub struct Direnv {
    allow: Allow,
    loaded: Option<Loaded>,
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
}

impl Direnv {
    pub fn new(xdg_data: Option<&str>, home: Option<&str>) -> Direnv {
        Direnv {
            allow: Allow::new(xdg_data, home),
            loaded: None,
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

    /// Drop whatever is loaded without touching the environment.
    ///
    /// For `direnv reload`, which wants the next directory check to do the work again from scratch.
    pub fn forget(&mut self) {
        self.loaded = None;
        self.told.clear();
    }

    /// Bring the environment into line with `dir`. The whole feature, from the caller's side.
    pub fn arrive(&mut self, env: &mut Environment, dir: &Path) -> Vec<Event> {
        let rcs = find::applicable(dir);
        let owner = find::owner(&rcs);

        // Standing in the same environment, with nothing edited: the overwhelmingly common case.
        if let Some(loaded) = &self.loaded
            && owner.as_deref() == Some(loaded.owner.as_path())
            && !self.changed_underneath()
        {
            return Vec::new();
        }

        let mut events = Vec::new();
        // Unload before load, always. Moving from one project to another must not carry anything
        // across, and doing this in the other order silently merges the two.
        if let Some(event) = self.unload(env) {
            events.push(event);
        }
        if rcs.is_empty() {
            return events;
        }

        if let Some(event) = self.load(env, &rcs) {
            events.push(event);
        }
        events
    }

    /// Whether any loaded file has been written since it was read.
    fn changed_underneath(&self) -> bool {
        let Some(loaded) = &self.loaded else {
            return false;
        };
        loaded
            .watches
            .iter()
            .any(|(path, when)| stamp(path) != *when)
    }

    /// Put back everything the loaded environment changed.
    fn unload(&mut self, env: &mut Environment) -> Option<Event> {
        let loaded = self.loaded.take()?;
        for (name, value) in loaded.undo.to_apply() {
            match value {
                Some(value) => {
                    env.set_var(name, value, true);
                }
                None => env.unset_var(name),
            }
        }
        Some(Event::Unloaded {
            owner: loaded.owner,
        })
    }

    /// Check the gate, then read.
    fn load(&mut self, env: &mut Environment, rcs: &[Rc]) -> Option<Event> {
        for rc in rcs {
            match self.allow.status(&rc.path) {
                Status::Denied => {
                    return Some(Event::Denied {
                        path: rc.path.clone(),
                    });
                }
                Status::NotAllowed => {
                    // Reported once per path. Returning here rather than loading the *other* files
                    // is deliberate: `.envrc` and `.env.lua` in one directory are one statement of
                    // intent, and loading half of it would produce an environment the author never
                    // described.
                    if self.told.contains(&rc.path) {
                        return None;
                    }
                    self.told.push(rc.path.clone());
                    return Some(Event::Blocked {
                        path: rc.path.clone(),
                    });
                }
                Status::Allowed => {}
            }
        }

        let before = snapshot(env);
        let watches = rcs
            .iter()
            .map(|rc| (rc.path.clone(), stamp(&rc.path)))
            .collect();
        for rc in rcs {
            apply(env, rc);
        }
        let after = snapshot(env);
        let undo = Diff::between(&before, &after).reverse();

        let owner = find::owner(rcs)?;
        let vars = undo.len();
        self.loaded = Some(Loaded {
            owner: owner.clone(),
            watches,
            undo,
        });
        Some(Event::Loaded { owner, vars })
    }
}

/// Every exported variable, which is what a directory environment is about.
fn snapshot(env: &Environment) -> BTreeMap<String, String> {
    env.exported_vars().into_iter().collect()
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

/// Read one rc file into the environment.
///
/// `.envrc` is not handled here: running shell means parsing and evaluating, which needs the
/// executor, and wiring that in from the library layer would make this module depend on half the
/// shell. The caller supplies it — see [`shell_source`].
fn apply(env: &mut Environment, rc: &Rc) {
    match rc.kind {
        Kind::Dotenv => {
            let Ok(source) = std::fs::read_to_string(&rc.path) else {
                return;
            };
            for (name, value) in dotenv::parse(&source) {
                env.set_var(&name, &value, true);
            }
        }
        // Both are handed to the caller, which owns an evaluator for each language.
        Kind::Shell | Kind::Lua => {}
    }
}

/// The rc files of a kind the caller has to evaluate, in load order.
///
/// `.envrc` needs the shell executor and `.env.lua` needs the Lua engine; neither belongs to this
/// module, so it reports what to run and the read loop runs it.
pub fn needs_evaluating(dir: &Path, allow: &Allow) -> Vec<Rc> {
    let rcs = find::applicable(dir);
    if rcs
        .iter()
        .any(|rc| allow.status(&rc.path) != Status::Allowed)
    {
        return Vec::new();
    }
    rcs.into_iter()
        .filter(|rc| matches!(rc.kind, Kind::Shell | Kind::Lua))
        .collect()
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
    fn shell() -> Environment {
        Environment::new()
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
        rc_in(project.path(), ".env", "SECRET=leaked\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        let mut env = shell();

        let events = direnv.arrive(&mut env, project.path());
        assert!(matches!(events.as_slice(), [Event::Blocked { .. }]));
        assert_eq!(env.get_var("SECRET"), None, "refused means not read");

        // The second arrival says nothing: a warning shown on every prompt is a warning nobody
        // reads, which is the worst outcome for this particular warning.
        direnv.loaded = None;
        assert!(direnv.arrive(&mut env, project.path()).is_empty());
    }

    /// The whole point: arriving sets, leaving restores.
    #[test]
    fn arriving_loads_and_leaving_puts_everything_back() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let elsewhere = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), ".env", "DATABASE_URL=postgres://local\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let mut env = shell();
        env.set_var("EDITOR", "vim", true);

        direnv.arrive(&mut env, project.path());
        assert_eq!(env.get_var("DATABASE_URL"), Some("postgres://local"));

        direnv.arrive(&mut env, elsewhere.path());
        assert_eq!(
            env.get_var("DATABASE_URL"),
            None,
            "leaving must remove it, not blank it"
        );
        assert_eq!(env.get_var("EDITOR"), Some("vim"), "and touch nothing else");
    }

    /// Moving straight from one project to another must not merge them.
    #[test]
    fn one_project_never_leaks_into_the_next() {
        let store = tempfile::tempdir().expect("temp dir");
        let first = tempfile::tempdir().expect("temp dir");
        let second = tempfile::tempdir().expect("temp dir");
        let one = rc_in(first.path(), ".env", "ONLY_IN_FIRST=1\n");
        let two = rc_in(second.path(), ".env", "ONLY_IN_SECOND=2\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&one).expect("allow");
        direnv.permissions().allow(&two).expect("allow");
        let mut env = shell();

        direnv.arrive(&mut env, first.path());
        direnv.arrive(&mut env, second.path());

        assert_eq!(env.get_var("ONLY_IN_SECOND"), Some("2"));
        assert_eq!(
            env.get_var("ONLY_IN_FIRST"),
            None,
            "unload has to happen before load, or the two environments merge"
        );
    }

    /// Standing still costs nothing, which is what makes this affordable on every `cd`.
    #[test]
    fn staying_put_does_no_work() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), ".env", "OSLO_T_STAY=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let mut env = shell();

        assert!(!direnv.arrive(&mut env, project.path()).is_empty());
        assert!(
            direnv.arrive(&mut env, project.path()).is_empty(),
            "a second arrival in the same place is not a reload"
        );
    }

    /// A subdirectory of the project is still the project.
    #[test]
    fn walking_deeper_stays_loaded() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let deep = project.path().join("src/inner");
        std::fs::create_dir_all(&deep).expect("mkdir");
        let path = rc_in(project.path(), ".env", "OSLO_T_DEEP=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let mut env = shell();

        direnv.arrive(&mut env, project.path());
        assert!(direnv.arrive(&mut env, &deep).is_empty());
        assert_eq!(env.get_var("OSLO_T_DEEP"), Some("1"));
    }

    /// Editing an allowed file revokes it, so the next arrival must refuse rather than reload.
    #[test]
    fn an_edit_revokes_and_the_environment_comes_back_out() {
        let store = tempfile::tempdir().expect("temp dir");
        let project = tempfile::tempdir().expect("temp dir");
        let path = rc_in(project.path(), ".env", "OSLO_T_EDIT_A=1\n");

        let mut direnv = Direnv::new(store.path().to_str(), None);
        direnv.permissions().allow(&path).expect("allow");
        let mut env = shell();
        direnv.arrive(&mut env, project.path());
        assert_eq!(env.get_var("OSLO_T_EDIT_A"), Some("1"));

        // Rewrite it. The mtime moves, so the next arrival re-checks, and the hash no longer matches.
        std::thread::sleep(std::time::Duration::from_millis(10));
        rc_in(project.path(), ".env", "OSLO_T_EDIT_A=1\nOSLO_T_EDIT_B=2\n");

        let events = direnv.arrive(&mut env, project.path());
        assert!(
            events.iter().any(|e| matches!(e, Event::Blocked { .. })),
            "an edited file has to be allowed again: {events:?}"
        );
        assert_eq!(
            env.get_var("OSLO_T_EDIT_A"),
            None,
            "and the old values come back out"
        );
        assert_eq!(
            env.get_var("OSLO_T_EDIT_B"),
            None,
            "the new ones never went in"
        );
    }
}
