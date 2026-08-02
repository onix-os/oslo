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
use std::sync::Mutex;
use std::time::SystemTime;

/// The shell's one directory-environment state.
///
/// A process global, like the tracking store, because two callers need it and neither owns the
/// other: the read loop drives the lifecycle on every `cd`, and the `direnv` builtin reads and
/// writes the allow list. Threading it through the builtin signature would change every builtin's
/// type for the sake of one of them.
static STATE: Mutex<Option<Direnv>> = Mutex::new(None);

/// Hand the shell its directory-environment state. Called once at startup.
pub fn install(direnv: Direnv) {
    if let Ok(mut slot) = STATE.lock() {
        *slot = Some(direnv);
    }
}

/// Do something with the state, if this shell has any.
///
/// Answers `None` in a script: a non-interactive shell has no directory environment at all, for the
/// same reason it has no tracking store. A script's environment comes from whoever ran it, and a
/// file in the working directory silently changing it would make scripts depend on where they are
/// invoked from.
pub fn with<T>(f: impl FnOnce(&mut Direnv) -> T) -> Option<T> {
    let mut slot = STATE.lock().ok()?;
    slot.as_mut().map(f)
}

/// Whether this shell has a directory environment at all.
pub fn installed() -> bool {
    STATE.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

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
    ///
    /// `run` evaluates one `.envrc` or `.env.lua` and reports what went wrong. It is a callback
    /// rather than something this module does, because running shell needs the executor and running
    /// Lua needs the engine, and neither belongs to a module about directories. It is called
    /// *inside* the before/after snapshot, which is what makes a variable an `.envrc` exports part
    /// of the undo record — evaluating outside the window would set variables nothing could unset.
    pub fn arrive(
        &mut self,
        env: &mut Environment,
        dir: &Path,
        run: &mut dyn FnMut(&mut Environment, &Rc) -> Result<(), String>,
    ) -> Vec<Event> {
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

        events.extend(self.load(env, &rcs, run));
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
    fn load(
        &mut self,
        env: &mut Environment,
        rcs: &[Rc],
        run: &mut dyn FnMut(&mut Environment, &Rc) -> Result<(), String>,
    ) -> Vec<Event> {
        for rc in rcs {
            match self.allow.status(&rc.path) {
                Status::Denied => {
                    return vec![Event::Denied {
                        path: rc.path.clone(),
                    }];
                }
                Status::NotAllowed => {
                    // Reported once per path. Returning here rather than loading the *other* files
                    // is deliberate: `.envrc` and `.env.lua` in one directory are one statement of
                    // intent, and loading half of it would produce an environment the author never
                    // described.
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
        }

        let before = snapshot(env);
        let watches = rcs
            .iter()
            .map(|rc| (rc.path.clone(), stamp(&rc.path)))
            .collect();
        let mut failures = Vec::new();
        for rc in rcs {
            let outcome = match rc.kind {
                Kind::Dotenv => apply_dotenv(env, rc),
                // Shell and Lua are the caller's to run.
                Kind::Shell | Kind::Lua => run(env, rc),
            };
            // **A file that failed half way still had its first half take effect**, so the diff is
            // taken over everything regardless and the failure is reported beside it. Discarding
            // the load here would leave those variables set with nothing recorded to unset them.
            if let Err(problem) = outcome {
                failures.push(Event::Failed {
                    path: rc.path.clone(),
                    problem,
                });
            }
        }
        let after = snapshot(env);
        let undo = Diff::between(&before, &after).reverse();

        let Some(owner) = find::owner(rcs) else {
            return failures;
        };
        let vars = undo.len();
        self.loaded = Some(Loaded {
            owner: owner.clone(),
            watches,
            undo,
        });
        let mut events = vec![Event::Loaded { owner, vars }];
        events.extend(failures);
        events
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

/// Read a `.env` into the environment. The only kind this module can read by itself.
fn apply_dotenv(env: &mut Environment, rc: &Rc) -> Result<(), String> {
    let source = std::fs::read_to_string(&rc.path).map_err(|e| e.to_string())?;
    for (name, value) in dotenv::parse(&source) {
        env.set_var(&name, &value, true);
    }
    Ok(())
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

    /// These tests use `.env` files only, which this module reads itself, so nothing needs running.
    fn nothing(_: &mut Environment, _: &Rc) -> Result<(), String> {
        Ok(())
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

        let events = direnv.arrive(&mut env, project.path(), &mut nothing);
        assert!(matches!(events.as_slice(), [Event::Blocked { .. }]));
        assert_eq!(env.get_var("SECRET"), None, "refused means not read");

        // The second arrival says nothing: a warning shown on every prompt is a warning nobody
        // reads, which is the worst outcome for this particular warning.
        direnv.loaded = None;
        assert!(
            direnv
                .arrive(&mut env, project.path(), &mut nothing)
                .is_empty()
        );
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

        direnv.arrive(&mut env, project.path(), &mut nothing);
        assert_eq!(env.get_var("DATABASE_URL"), Some("postgres://local"));

        direnv.arrive(&mut env, elsewhere.path(), &mut nothing);
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

        direnv.arrive(&mut env, first.path(), &mut nothing);
        direnv.arrive(&mut env, second.path(), &mut nothing);

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

        assert!(
            !direnv
                .arrive(&mut env, project.path(), &mut nothing)
                .is_empty()
        );
        assert!(
            direnv
                .arrive(&mut env, project.path(), &mut nothing)
                .is_empty(),
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

        direnv.arrive(&mut env, project.path(), &mut nothing);
        assert!(direnv.arrive(&mut env, &deep, &mut nothing).is_empty());
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
        direnv.arrive(&mut env, project.path(), &mut nothing);
        assert_eq!(env.get_var("OSLO_T_EDIT_A"), Some("1"));

        // Rewrite it. The mtime moves, so the next arrival re-checks, and the hash no longer matches.
        std::thread::sleep(std::time::Duration::from_millis(10));
        rc_in(project.path(), ".env", "OSLO_T_EDIT_A=1\nOSLO_T_EDIT_B=2\n");

        let events = direnv.arrive(&mut env, project.path(), &mut nothing);
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
