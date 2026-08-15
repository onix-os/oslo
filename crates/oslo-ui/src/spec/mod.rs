//! Completion specs: what to suggest after a command name.
//!
//! [`SpecRegistry`] holds one [`CommandSpec`] per known command; the per-command data lives
//! in [`definitions`], and ranking uses [`FrecencyTracker`].
//!
//! # Owned strings, since a config can declare one
//!
//! Every field here used to be `&'static str`, which is the natural shape for four hand-written
//! definitions compiled into the binary and an impossible one for a spec built at runtime from Lua.
//! That single word was the whole reason a plugin's only route to completion was `for_command` — a
//! function that has to re-implement subcommand matching, flag parsing and descriptions by hand.
//!
//! The cost is an allocation per string at *build* time and a pointer chase at *read* time. Both
//! were measured on a Tab against `git comm`, which is the deepest walk of this data the shell does:
//! see `docs/features/completion-and-matching.md`.

pub mod custom;
pub mod definitions;
pub mod frecency;

pub use frecency::FrecencyTracker;

use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct OptionSpec {
    /// Every spelling of one flag — `["-m", "--message"]`. All of them are offered.
    pub names: Vec<String>,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct SubcommandSpec {
    pub name: String,
    pub description: String,
    pub subcommands: Vec<SubcommandSpec>,
    pub options: Vec<OptionSpec>,
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub name: String,
    pub description: String,
    pub subcommands: Vec<SubcommandSpec>,
    pub options: Vec<OptionSpec>,
}

pub struct SpecRegistry {
    specs: HashMap<String, Rc<CommandSpec>>,
    pub frecency: FrecencyTracker,
}

impl Default for SpecRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SpecRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            specs: HashMap::new(),
            frecency: FrecencyTracker::new(),
        };
        for spec in definitions::all() {
            registry.register(spec);
        }
        registry
    }

    pub fn register(&mut self, spec: CommandSpec) {
        self.specs.insert(spec.name.clone(), Rc::new(spec));
    }

    /// The spec for `cmd`, if anything has one.
    ///
    /// **A config's spec wins over a built-in one.** The four compiled in are a starting point, not
    /// a claim to be right forever — `git` grows subcommands faster than this tree does — and
    /// somebody who has written a better one should get theirs. That is the same rule the settings
    /// take, and the opposite of `register_tool`, where a name that already means something keeps
    /// its meaning because a *command* changing under a script is a different kind of surprise.
    ///
    /// `Rc` rather than a reference because the config-declared table cannot lend one out: it lives
    /// in a `RefCell` that has to be given back before this returns. Cloning the `Rc` is a counter
    /// bump, where cloning the spec would copy git's whole subcommand tree on every keystroke.
    pub fn find_spec(&self, cmd: &str) -> Option<Rc<CommandSpec>> {
        custom::find(cmd).or_else(|| self.specs.get(cmd).cloned())
    }
}
