//! The trap table: what a condition is bound to, and what a listing may report.
//!
//! Split from `scope.rs` because the two maps here answer different questions and keeping them
//! next to each other is the point — `signal_traps` is what *runs*, `inherited_traps` is what a
//! subshell may still *report*. Mixing them is what made `saved=$(trap)` come back empty.

use super::Environment;
use std::collections::HashMap;

impl Environment {
    pub fn set_trap(&mut self, sig: &str, handler: &str) {
        let key = sig.to_uppercase();
        // A trap set here supersedes whatever was inherited, so the old text must not go on being
        // listed beside the new one.
        self.inherited_traps.remove(&key);
        self.signal_traps.insert(key, handler.to_string());
    }

    pub fn get_trap(&self, sig: &str) -> Option<&str> {
        self.signal_traps
            .get(&sig.to_uppercase())
            .map(|s| s.as_str())
    }

    pub fn get_traps(&self) -> &HashMap<String, String> {
        &self.signal_traps
    }

    /// Every trap `trap` should print: this shell's, plus any it inherited and reset.
    ///
    /// Deliberately *not* what the dispatcher reads. A subshell runs none of the inherited
    /// handlers — that is what resetting means — but `saved=$(trap)` must still see them, or the
    /// save-and-restore idiom silently saves nothing.
    pub fn listable_traps(&self) -> HashMap<String, String> {
        let mut all = self.inherited_traps.clone();
        all.extend(
            self.signal_traps
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        all
    }
}
