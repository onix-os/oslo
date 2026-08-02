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
    /// The variables to put back on the way out, each with the export flag it had.
    undo: Diff<(String, bool)>,
    /// The aliases to put back on the way out.
    ///
    /// Separate from `undo` rather than merged into it because an alias and a variable can share a
    /// name and mean different things — `ls` is a perfectly good alias and a perfectly good
    /// variable, and one map would have them overwrite each other.
    aliases: Diff<String>,
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

    /// The names the loaded environment is holding — variables first, then aliases.
    pub fn holding(&self) -> Vec<&str> {
        let Some(loaded) = self.loaded.as_ref() else {
            return Vec::new();
        };
        let mut names = loaded.undo.names();
        names.extend(loaded.aliases.names());
        names
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
    /// `restore` puts back whatever the caller recorded at load time and this module cannot
    /// describe — the Lua prompt. Called on the way out, before the variables are restored, and
    /// always *before* `run`, which is what makes moving straight from one project to another put
    /// the base prompt back before the next file gets to set its own.
    ///
    /// `run` evaluates the `.env.lua` and reports what went wrong. A callback rather than something
    /// this module does, because running Lua needs the engine and that belongs to the read loop.
    ///
    /// **The environment lock is not held while it runs, and must not be.** The file's whole job is
    /// to call `oslo.env.set` and friends, and those take the same lock with `try_lock` — holding it
    /// here made every one of them fail with "shell state is busy" while the load reported success.
    /// So the lock is taken twice, briefly, for the before and after snapshots, and released around
    /// the call in between.
    pub fn arrive(
        &mut self,
        env: &Mutex<Environment>,
        dir: &Path,
        run: &mut dyn FnMut(&Rc) -> Result<(), String>,
        restore: &mut dyn FnMut(),
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
        if let Some(event) = self.unload(env, restore) {
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
    fn unload(&mut self, env: &Mutex<Environment>, restore: &mut dyn FnMut()) -> Option<Event> {
        let loaded = self.loaded.take()?;
        // Anything the caller has to put back that this module cannot describe — the Lua prompt,
        // today. Run before the variables so a prompt function that reads one sees the directory's
        // value while it is still set, rather than the restored one it is about to be handed.
        restore();
        let mut guard = lock(env)?;
        for (name, was) in loaded.undo.to_apply() {
            match was {
                // The export flag is restored with the value. A variable that was shell-local
                // before the directory exported it has to come back local, or leaving would hand
                // every later child a variable that was never in its environment.
                Some((value, exported)) => {
                    // Unset first. `set_var`'s export flag is OR'd with whatever the variable
                    // already carries, so setting it to `false` on a currently-exported variable
                    // does nothing — the only way back to shell-local is to clear the entry and
                    // write it again.
                    guard.unset_var(name);
                    guard.set_var(name, value, *exported);
                }
                None => guard.unset_var(name),
            }
        }
        for (name, value) in loaded.aliases.to_apply() {
            match value {
                Some(value) => guard.set_alias(name, value),
                None => guard.remove_alias(name),
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

        let Some((before, aliases_before)) = lock(env).map(|guard| shell_state(&guard)) else {
            return Vec::new();
        };
        let watch = (rc.path.clone(), stamp(&rc.path));
        // No lock held here. See the note on `arrive`.
        let outcome = run(rc);
        let Some((after, aliases_after)) = lock(env).map(|guard| shell_state(&guard)) else {
            return Vec::new();
        };
        // **A file that failed half way still had its first half take effect**, so the diff is
        // taken regardless and the failure reported beside it. Discarding the load here would
        // leave those variables set with nothing recorded to unset them.
        let undo = Diff::between(&before, &after).reverse();
        let aliases = Diff::between(&aliases_before, &aliases_after).reverse();

        let Some(owner) = find::owner(rc) else {
            return Vec::new();
        };
        let vars = undo.len() + aliases.len();
        self.loaded = Some(Loaded {
            owner: owner.clone(),
            watch,
            undo,
            aliases,
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

/// Everything a `.env.lua` can set that leaving must put back: `(exported variables, aliases)`.
///
/// Taken as one call so both halves are read under the same lock, which is what makes the before
/// and after snapshots describe the same instant.
type Vars = BTreeMap<String, (String, bool)>;

fn shell_state(env: &Environment) -> (Vars, BTreeMap<String, String>) {
    (
        env.all_vars()
            .into_iter()
            .map(|(name, value, exported)| (name, (value, exported)))
            .collect(),
        env.get_aliases().clone().into_iter().collect(),
    )
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
mod tests;
