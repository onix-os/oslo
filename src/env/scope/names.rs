//! The names the shell knows besides variables: aliases, functions and builtins.
//!
//! A second `impl Environment` in a child module, which sees the private fields because it is a
//! descendant of the one that declares them. Split out because `scope.rs` crossed the 600-line
//! limit and this is the seam that means something: everything here answers "what does this word
//! resolve to", where the rest of `scope.rs` answers "what state is this shell in".

use super::*;

impl Environment {
    pub fn set_alias(&mut self, name: &str, value: &str) {
        self.aliases.insert(name.to_string(), value.to_string());
    }

    pub fn get_alias(&self, name: &str) -> Option<&str> {
        self.aliases.get(name).map(|s| s.as_str())
    }

    pub fn get_aliases(&self) -> &HashMap<String, String> {
        &self.aliases
    }

    pub fn remove_alias(&mut self, name: &str) {
        self.aliases.remove(name);
    }

    pub fn set_function(&mut self, name: &str, body: Command) {
        self.functions.insert(name.to_string(), body);
    }

    /// Forget the function `name`; `true` if there was one.
    ///
    /// `unset -f` is the only caller, and without this it had nothing to call: the functions
    /// table was exposed read-only, so a function could be defined and never taken back.
    /// The answer is not `unset`'s exit status — bash exits 0 for `unset -f nosuchfn` — but it
    /// is what a caller that needs to know would ask for.
    pub fn remove_function(&mut self, name: &str) -> bool {
        self.functions.remove(name).is_some()
    }

    pub fn get_function(&self, name: &str) -> Option<&Command> {
        self.functions.get(name)
    }

    pub fn get_functions(&self) -> &HashMap<String, Command> {
        &self.functions
    }

    /// Register `name` as a builtin, replacing any earlier one.
    ///
    /// The single point of entry: whatever is registered here is what `is_builtin`, the
    /// dispatcher, completion and `type` will all report and run.
    pub fn register_custom_builtin(&mut self, name: &str, func: BuiltinFn) {
        self.builtins.register(name, func);
    }

    /// The implementation registered for `name`, if any. Prefer [`Self::exec_custom_builtin`]:
    /// this exists for callers that need to know a builtin is callable without calling it.
    pub fn get_builtin(&self, name: &str) -> Option<BuiltinFn> {
        self.builtins.lookup(name)
    }

    /// Every registered builtin name.
    ///
    /// The single source of truth for completion and `type`, so adding a builtin does not
    /// require remembering to update a list somewhere else.
    pub fn builtin_names(&self) -> impl Iterator<Item = &str> {
        self.builtins.names()
    }

    /// Whether `name` is a builtin — that is, whether it is in the registry.
    ///
    /// This used to be a `matches!` over a hand-written list that had drifted from both the
    /// registry and the dispatcher's own `match` (PLAN R5.6): `type` called `set` an external
    /// command while the shell ran it as a builtin, and a name in the list with no
    /// implementation dispatched to `Ok(0)` — a command that silently did nothing.
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtins.contains(name)
    }
}
