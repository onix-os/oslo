//! Completion specs: what to suggest after a command name.
//!
//! [`SpecRegistry`] holds one [`CommandSpec`] per known command; the per-command data lives
//! in [`definitions`], and ranking uses [`FrecencyTracker`].

pub mod definitions;
pub mod frecency;

pub use frecency::FrecencyTracker;

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct OptionSpec {
    pub names: Vec<&'static str>,
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct SubcommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub subcommands: Vec<SubcommandSpec>,
    pub options: Vec<OptionSpec>,
}

#[derive(Debug, Clone)]
pub struct CommandSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub subcommands: Vec<SubcommandSpec>,
    pub options: Vec<OptionSpec>,
}

pub struct SpecRegistry {
    specs: HashMap<&'static str, CommandSpec>,
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
        self.specs.insert(spec.name, spec);
    }

    pub fn find_spec(&self, cmd: &str) -> Option<&CommandSpec> {
        self.specs.get(cmd)
    }

    pub fn get_subcommand_suggestions(
        &self,
        cmd: &str,
        tokens: &[&str],
        word: &str,
    ) -> Vec<(&'static str, &'static str)> {
        let mut results = Vec::new();
        if let Some(spec) = self.find_spec(cmd) {
            if tokens.len() <= 1 {
                // Suggest subcommands of top-level command
                for sub in &spec.subcommands {
                    if sub.name.starts_with(word) {
                        results.push((sub.name, sub.description));
                    }
                }
                // Suggest top-level options
                for opt in &spec.options {
                    for name in &opt.names {
                        if name.starts_with(word) {
                            results.push((name, opt.description));
                        }
                    }
                }
            } else {
                // Tokens length > 1 (e.g., `git commit -`)
                let sub_name = tokens[0];
                if let Some(sub) = spec.subcommands.iter().find(|s| s.name == sub_name) {
                    if word.starts_with('-') {
                        for opt in &sub.options {
                            for name in &opt.names {
                                if name.starts_with(word) {
                                    results.push((name, opt.description));
                                }
                            }
                        }
                    } else {
                        for nested in &sub.subcommands {
                            if nested.name.starts_with(word) {
                                results.push((nested.name, nested.description));
                            }
                        }
                    }
                }
            }
        }
        results
    }
}
