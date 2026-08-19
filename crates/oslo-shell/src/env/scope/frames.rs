//! Dynamic scoping: the frames `local` and a function call push and pop.
//!
//! A frame does not *hold* the shell's variables — it holds what they looked like before the frame
//! touched them, so popping restores rather than merges. That is what makes `local` dynamic
//! scoping rather than lexical: a callee sees the caller's value until it declares its own.
//!
//! Each name is snapshotted in **both** shapes it could have, scalar and array, because a name is
//! only ever one of the two and an assignment may change which. Saving only the scalar half is
//! what would let `f() { local a; a=(1 2); }` leave a global array behind after `f` returned.

use super::{Environment, ShellArray, environ_remove, environ_set, reject_unrepresentable};
use std::collections::HashMap;

/// One `local` scope: the previous value of every name the scope has written to.
///
/// `None` against a name means "did not exist", which pop must turn back into "does not exist"
/// rather than into an empty string.
#[derive(Default)]
pub(super) struct ScopeFrame {
    vars: HashMap<String, Option<(String, bool)>>,
    arrays: HashMap<String, Option<ShellArray>>,
}

impl Environment {
    pub fn push_scope(&mut self) {
        self.scope_stack.push(ScopeFrame::default());
    }

    pub fn pop_scope(&mut self) {
        let Some(frame) = self.scope_stack.pop() else {
            return;
        };
        for (name, original) in frame.arrays {
            match original {
                Some(array) => {
                    self.arrays.insert(name, array);
                }
                None => {
                    self.arrays.remove(&name);
                }
            }
        }
        for (name, original) in frame.vars {
            match original {
                Some((value, exported)) => {
                    self.vars.insert(name.clone(), (value.clone(), exported));
                    if exported {
                        environ_set(&name, &value);
                    } else {
                        // The variable existed but was not exported. If it was exported
                        // temporarily inside this scope, the process environment still holds it
                        // and would leak into every later child.
                        environ_remove(&name);
                    }
                }
                None => {
                    self.vars.remove(&name);
                    environ_remove(&name);
                }
            }
        }
    }

    /// Remember `name`'s current value in the innermost scope so [`Self::pop_scope`] restores it.
    pub(super) fn save_for_restore(&mut self, name: &str) {
        let scalar = self.vars.get(name).cloned();
        let array = self.arrays.get(name).cloned();
        if let Some(frame) = self.scope_stack.last_mut() {
            frame.vars.entry(name.to_string()).or_insert(scalar);
            frame.arrays.entry(name.to_string()).or_insert(array);
        }
    }

    /// Set `name` in the innermost scope. `false` if the assignment was refused.
    pub fn set_local_var(&mut self, name: &str, value: &str) -> bool {
        // Checked before `save_for_restore`: a name recorded in the frame is passed to
        // `environ_remove` when the scope pops, so an unusable one must never get in there.
        if reject_unrepresentable(&self.origin(), name, value) {
            return false;
        }
        self.save_for_restore(name);
        self.set_var(name, value, false)
    }

    /// Set a variable that is exported for the lifetime of the innermost scope only.
    ///
    /// This is what a command-prefix assignment (`FOO=bar cmd`) needs: `cmd` must see `FOO` in
    /// its environment, and the shell must not.
    pub fn set_local_exported_var(&mut self, name: &str, value: &str) -> bool {
        if reject_unrepresentable(&self.origin(), name, value) {
            return false;
        }
        self.save_for_restore(name);
        self.set_var(name, value, true)
    }

    /// Make `name` an array in the innermost scope — `local -a a`, and `a=(…)` under a `local`
    /// declaration. Restored wholesale when the frame pops.
    pub fn set_local_array(&mut self, name: &str, array: ShellArray) -> bool {
        self.save_for_restore(name);
        self.set_array(name, array)
    }
}

#[cfg(test)]
mod tests {
    use crate::env::Environment;
    use crate::env::scope::ShellArray;

    /// A local array must not outlive its frame — and must not leave the *scalar* of the same
    /// name behind either, since assigning an array discards it.
    #[test]
    fn a_local_array_is_undone_by_its_frame() {
        let mut env = Environment::new();
        env.set_var("oslo_f1", "outer", false);
        env.push_scope();
        env.set_local_array("oslo_f1", ShellArray::from_values(["1", "2"]));
        assert_eq!(env.get_var("oslo_f1"), Some("1"));
        env.pop_scope();
        assert!(env.get_array("oslo_f1").is_none());
        assert_eq!(env.get_var("oslo_f1"), Some("outer"));
    }

    /// And an array that only *existed* inside the frame is gone afterwards.
    #[test]
    fn a_local_array_with_no_outer_value_disappears() {
        let mut env = Environment::new();
        env.push_scope();
        env.set_local_array("oslo_f2", ShellArray::from_values(["x"]));
        env.pop_scope();
        assert!(env.get_array("oslo_f2").is_none());
        assert_eq!(env.get_var("oslo_f2"), None);
    }
}
