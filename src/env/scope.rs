mod array;
mod frames;
mod options;
mod registry;
#[cfg(test)]
mod tests;

use crate::ast::Command;
use crate::env::nesting::{DepthGuard, MAX_FUNCTION_DEPTH, MAX_SCRIPT_DEPTH};
use crate::env::options::ShellOptions;
use crate::error::Result;
use frames::ScopeFrame;
use registry::BuiltinRegistry;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;

pub use array::{ShellArray, array_literal_body};
pub use registry::{BuiltinFn, is_special_builtin};

/// Whether `name` is a valid shell variable name: `[A-Za-z_][A-Za-z0-9_]*`.
///
/// `export`, `local` and `readonly` reject anything else *before* it can reach the process
/// environment, because `std::env::set_var` panics on a name the OS refuses (empty, or
/// containing `=`) and a panic in the interpreter loop takes an interactive session with it.
pub fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `name`/`value` can be handed to the process environment without aborting.
///
/// Deliberately weaker than [`is_valid_identifier`]: names inherited from `environ` may contain
/// characters no shell would accept (`BASH_FUNC_x%%`), and forwarding those to a child is
/// correct. Only the three things `environ` genuinely cannot represent are refused — an empty
/// name, a `=` in a name, and a NUL anywhere, since NUL terminates a C string.
fn is_environ_safe(name: &str, value: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0']) && !value.contains('\0')
}

/// Report a name/value the process environment cannot represent; `true` if it was rejected.
///
/// The last line of defence for callers that reach [`Environment::set_var`] without doing their
/// own validation (`read`, a `for` loop variable, a `${x=default}` expansion): the assignment is
/// dropped with a diagnostic rather than aborting the shell.
fn reject_unrepresentable(name: &str, value: &str) -> bool {
    if name.is_empty() || name.contains(['=', '\0']) {
        eprintln!("rush: {}: not a valid identifier", name);
        true
    } else if value.contains('\0') {
        eprintln!("rush: {}: value contains a NUL byte", name);
        true
    } else {
        false
    }
}

/// Publish `name=value` to the process environment; a pair `environ` cannot hold is dropped.
///
/// Every write to `environ` in this module funnels through here so the soundness argument lives
/// in exactly one place.
fn environ_set(name: &str, value: &str) {
    if !is_environ_safe(name, value) {
        return;
    }
    // SAFETY: `std::env::set_var` is unsafe in edition 2024 because it mutates the global
    // `environ` with no synchronisation, so a concurrent `getenv` in another thread could read a
    // freed pointer. rush is single-threaded: nothing in the crate spawns a thread, the parser,
    // interpreter, builtins and Lua engine all run on the main thread, and a forked child starts
    // with only the forking thread alive. The guard above rules out the other failure mode — the
    // call panics on an empty name, a `=` in the name, or a NUL in either half.
    unsafe { env::set_var(name, value) }
}

/// Drop `name` from the process environment; a name `environ` cannot hold is ignored.
fn environ_remove(name: &str) {
    if !is_environ_safe(name, "") {
        return;
    }
    // SAFETY: as in `environ_set` — no other thread exists to observe the mutation, and the
    // guard above excludes the names `remove_var` panics on.
    unsafe { env::remove_var(name) }
}

pub struct Environment {
    vars: HashMap<String, (String, bool)>, // (value, is_exported)
    /// Indexed arrays, in the same namespace as [`Self::vars`]: a name is a scalar or an array,
    /// never both. See the `array` submodule for why the two tables stay separate.
    arrays: HashMap<String, ShellArray>,
    positional: Vec<String>,
    pub last_status: i32,
    pub pid: u32,
    pub last_bg_pid: Option<u32>,
    pub shell_name: String,
    /// This process's own pid, which differs from [`Self::pid`] inside a forked subshell: `$$`
    /// keeps reporting the *invoking* shell (POSIX; `bash -c 'echo $$; (echo $$)'` prints one
    /// number twice), so job control and `$BASHPID` need the real one kept separately.
    current_pid: u32,
    /// Exit status of every stage of the most recent pipeline, left to right. Published as the
    /// `PIPESTATUS` array by [`Self::set_pipeline_status`]; `pipefail` reads the same vector.
    pipeline_status: Vec<i32>,
    /// Exit status of the last command substitution, until something consumes it.
    substitution_status: Option<i32>,
    aliases: HashMap<String, String>,
    functions: HashMap<String, Command>,
    /// Every builtin this shell has. The one list consulted by [`Self::is_builtin`], the
    /// dispatcher in `exec::simple` and `type`; see the `registry` submodule.
    builtins: BuiltinRegistry,
    signal_traps: HashMap<String, String>,
    readonly_vars: HashSet<String>,
    dir_stack: Vec<PathBuf>,
    scope_stack: Vec<ScopeFrame>,
    /// How many loops are currently executing in this shell.
    ///
    /// `break` and `continue` unwind as errors, which would abandon the rest of the enclosing
    /// command list. Outside any loop they must instead do nothing at all — `break; echo hi`
    /// still prints `hi` — so the builtins consult this before signalling.
    loop_depth: usize,
    /// How deep the current shell-function call chain is.
    function_depth: DepthGuard,
    /// How deep the current `source`/`eval` chain is.
    script_depth: DepthGuard,
    /// The `set -e`/`set -o pipefail` options. Read through the accessors in the `options`
    /// submodule, which is where the whole option API lives.
    options: ShellOptions,
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl Environment {
    pub fn new() -> Self {
        let mut vars = HashMap::new();
        for (k, v) in env::vars() {
            vars.insert(k, (v, true));
        }

        let pid = std::process::id();

        let mut aliases = HashMap::new();
        aliases.insert("ll".to_string(), "ls -la".to_string());
        aliases.insert("la".to_string(), "ls -A".to_string());
        aliases.insert("l".to_string(), "ls -CF".to_string());

        let mut env_struct = Self {
            vars,
            arrays: HashMap::new(),
            positional: Vec::new(),
            last_status: 0,
            pid,
            last_bg_pid: None,
            shell_name: "rush".to_string(),
            current_pid: pid,
            pipeline_status: vec![0],
            substitution_status: None,
            aliases,
            functions: HashMap::new(),
            builtins: BuiltinRegistry::default(),
            signal_traps: HashMap::new(),
            readonly_vars: HashSet::new(),
            dir_stack: Vec::new(),
            scope_stack: Vec::new(),
            loop_depth: 0,
            function_depth: DepthGuard::new(MAX_FUNCTION_DEPTH),
            script_depth: DepthGuard::new(MAX_SCRIPT_DEPTH),
            options: ShellOptions::default(),
        };

        crate::env::builtins::register_default_builtins(&mut env_struct);
        env_struct
    }

    /// Mark this environment as the one a freshly forked subshell is running in.
    ///
    /// A subshell *is* the shell: `fork` already copied every variable with its export flag,
    /// every function, alias, positional parameter, `$0`, `$?`, readonly mark and the directory
    /// stack, so the child keeps what it inherited and only genuinely subshell-local state is
    /// refreshed here. Rebuilding an `Environment::new()` instead lost all of that *and*
    /// re-exported private variables into the child's `environ` — `x=1; (env | grep '^x=')`
    /// used to leak a variable the parent never exported.
    ///
    /// Refreshed: the recorded pid, and the traps, which POSIX resets to their default action in
    /// a subshell (`trap 'echo T' EXIT; (:)` prints `T` once). Deliberately kept: `$$`, the
    /// invoking shell's pid, and `$!`, which bash inherits into a subshell.
    pub fn enter_subshell(&mut self) {
        self.current_pid = std::process::id();
        self.signal_traps.clear();
    }

    /// Whether this environment belongs to a forked subshell rather than the top-level shell.
    pub fn in_subshell(&self) -> bool {
        self.current_pid != self.pid
    }

    /// This process's real pid, as opposed to `$$`. See [`Self::enter_subshell`].
    pub fn current_pid(&self) -> u32 {
        self.current_pid
    }

    /// Record every stage of a pipeline's exit status, left to right. A one-command pipeline
    /// records a single status, as bash's `PIPESTATUS` does.
    ///
    /// Publishing `PIPESTATUS` here rather than synthesising it during expansion keeps one copy
    /// of the numbers: `${PIPESTATUS[0]}` and `set -o pipefail` cannot disagree about what the
    /// last pipeline did. Written directly into the array table, bypassing the readonly check —
    /// this is the shell recording its own state, not a user assignment.
    pub fn set_pipeline_status(&mut self, statuses: Vec<i32>) {
        self.arrays.insert(
            "PIPESTATUS".to_string(),
            ShellArray::from_values(statuses.iter().map(i32::to_string)),
        );
        self.pipeline_status = statuses;
    }

    /// The stage statuses of the most recent pipeline. Never empty.
    pub fn pipeline_status(&self) -> &[i32] {
        &self.pipeline_status
    }

    /// Record what a command substitution exited with. Called by `exec::substitution`.
    pub fn note_substitution_status(&mut self, status: i32) {
        self.substitution_status = Some(status);
    }

    /// Take the status of the last command substitution, clearing it.
    ///
    /// What an assignment-only command reports (POSIX: `x=$(exit 5)` leaves `$?` at 5). Consumed
    /// rather than read, so a later assignment with no substitution reports 0, not a stale number.
    pub fn take_substitution_status(&mut self) -> Option<i32> {
        self.substitution_status.take()
    }

    pub fn enter_loop(&mut self) {
        self.loop_depth += 1;
    }

    pub fn exit_loop(&mut self) {
        self.loop_depth = self.loop_depth.saturating_sub(1);
    }

    /// Whether a `break` or `continue` has a loop to act on.
    pub fn in_loop(&self) -> bool {
        self.loop_depth > 0
    }

    /// Begin a shell-function call; `Err` once the call chain is too deep to be safe.
    ///
    /// The caller must pair this with [`Self::exit_function`] on every path out of the call,
    /// including an unwinding `return` or error.
    pub fn enter_function(&mut self) -> Result<()> {
        self.function_depth.enter()
    }

    pub fn exit_function(&mut self) {
        self.function_depth.exit()
    }

    /// Whether a shell function is currently executing.
    ///
    /// `local` needs this rather than the scope-frame stack, because a prefix assignment
    /// (`FOO=bar cmd`) pushes a frame too — so a non-empty stack does not mean "inside a
    /// function", and `local x=1` at the top level would silently create a global.
    pub fn in_function(&self) -> bool {
        self.function_depth.depth() > 0
    }

    /// Begin a nested `source` or `eval`; `Err` once the chain is too deep to be safe.
    pub fn enter_nested_script(&mut self) -> Result<()> {
        self.script_depth.enter()
    }

    pub fn exit_nested_script(&mut self) {
        self.script_depth.exit()
    }

    pub fn set_readonly(&mut self, name: &str) {
        self.readonly_vars.insert(name.to_string());
    }

    pub fn is_readonly(&self, name: &str) -> bool {
        self.readonly_vars.contains(name)
    }

    pub fn push_dir(&mut self, path: PathBuf) {
        self.dir_stack.push(path);
    }

    pub fn pop_dir(&mut self) -> Option<PathBuf> {
        self.dir_stack.pop()
    }

    pub fn get_dir_stack(&self) -> &[PathBuf] {
        &self.dir_stack
    }

    pub fn set_trap(&mut self, sig: &str, handler: &str) {
        self.signal_traps
            .insert(sig.to_uppercase(), handler.to_string());
    }

    pub fn get_trap(&self, sig: &str) -> Option<&str> {
        self.signal_traps
            .get(&sig.to_uppercase())
            .map(|s| s.as_str())
    }

    pub fn get_traps(&self) -> &HashMap<String, String> {
        &self.signal_traps
    }

    /// Run `name` as a builtin, or `None` if it is not one.
    ///
    /// The only way a builtin is ever invoked. A caller that has already asked
    /// [`Self::is_builtin`] still goes through here rather than through a name-to-function
    /// `match` of its own: the second list is what let `register_custom_builtin("echo", …)`
    /// register a function nothing would ever call (PLAN R5.6, R9.8).
    pub fn exec_custom_builtin(&mut self, name: &str, args: &[String]) -> Option<Result<i32>> {
        self.builtins.lookup(name).map(|func| func(self, args))
    }

    /// A variable's value as a single string.
    ///
    /// An array answers with its element 0, because in bash `$a` and `${a[0]}` are the same
    /// reference: `a=(1 2 3); echo "$a"` prints `1`.
    pub fn get_var(&self, name: &str) -> Option<&str> {
        match name {
            // `$-` joins the list: it is computed from the option bitset, and a variable of that
            // name (which no assignment can create, but `environ` can carry) must not shadow it.
            "?" | "$" | "!" | "#" | "*" | "@" | "-" => None,
            _ => match self.vars.get(name) {
                Some((v, _)) => Some(v.as_str()),
                None => self.arrays.get(name).and_then(|a| a.get(0)),
            },
        }
    }

    /// What `$*` joins positional parameters with: the first character of IFS, or nothing at all
    /// when IFS is set but empty. An unset IFS means the default, whose first character is a space.
    pub fn ifs_separator(&self) -> String {
        match self.get_var("IFS") {
            Some(ifs) => ifs.chars().next().map(String::from).unwrap_or_default(),
            None => " ".to_string(),
        }
    }

    pub fn get_param(&self, name: &str) -> Option<String> {
        match name {
            "?" => Some(self.last_status.to_string()),
            "$" => Some(self.pid.to_string()),
            "!" => self.last_bg_pid.map(|p| p.to_string()),
            "#" => Some(self.positional.len().to_string()),
            "-" => Some(self.option_flags()),
            "0" => Some(self.shell_name.clone()),
            // Only the forms that genuinely need a single string reach here: `"$@"` and `$@` are
            // resolved as a *field list* in `expand::param`, because collapsing them to one string
            // is what silently corrupted every wrapper script's arguments.
            "*" | "@" => Some(self.positional.join(&self.ifs_separator())),
            s => {
                if let Ok(idx) = s.parse::<usize>()
                    && idx > 0
                    && idx <= self.positional.len()
                {
                    return Some(self.positional[idx - 1].clone());
                }
                self.get_var(name).map(|s| s.to_string())
            }
        }
    }

    /// Assign `name=value`. `false` if the variable is read-only or unrepresentable.
    ///
    /// A scalar assignment to a name that already holds an array writes **element 0**, which is
    /// what bash does: `a=(1 2 3); a=4` leaves `${a[@]}` as `4 2 3`. Deciding that here rather
    /// than in the assignment code means `read a`, a `for a in …` loop variable and `${a:=x}` all
    /// agree about it for free.
    pub fn set_var(&mut self, name: &str, value: &str, export: bool) -> bool {
        if self.is_readonly(name) {
            eprintln!("rush: {}: is read only", name);
            return false;
        }
        if reject_unrepresentable(name, value) {
            return false;
        }
        if let Some(array) = self.arrays.get_mut(name) {
            array.set(0, value);
            return true;
        }
        let is_exp = export || self.vars.get(name).map(|(_, exp)| *exp).unwrap_or(false);
        self.vars
            .insert(name.to_string(), (value.to_string(), is_exp));
        if is_exp {
            environ_set(name, value);
        }
        true
    }

    /// Remove `name` entirely, in whichever shape it has. `unset a` drops the whole array, not
    /// just its element 0.
    pub fn unset_var(&mut self, name: &str) {
        self.vars.remove(name);
        self.arrays.remove(name);
        environ_remove(name);
    }

    /// Forget any *scalar* under `name`, leaving the array table alone.
    ///
    /// Used by the array store when a name changes shape; not public, because outside that
    /// transition "unset" always means [`Self::unset_var`].
    fn drop_scalar(&mut self, name: &str) {
        self.vars.remove(name);
        environ_remove(name);
    }

    /// Mark an existing variable exported, creating it empty if it does not exist.
    ///
    /// `false` if its current value cannot live in `environ` — a NUL-bearing value read from a
    /// binary file, say, which `export` must refuse instead of handing to `setenv`.
    pub fn export_var(&mut self, name: &str) -> bool {
        if let Some((val, _)) = self.vars.get(name) {
            let val = val.clone();
            if reject_unrepresentable(name, &val) {
                return false;
            }
            if let Some((_, exp)) = self.vars.get_mut(name) {
                *exp = true;
            }
            environ_set(name, &val);
        } else {
            if reject_unrepresentable(name, "") {
                return false;
            }
            self.vars.insert(name.to_string(), ("".to_string(), true));
            environ_set(name, "");
        }
        true
    }

    pub fn get_all_vars(&self) -> HashMap<String, String> {
        self.vars
            .iter()
            .map(|(k, (v, _))| (k.clone(), v.clone()))
            .collect()
    }

    pub fn get_exported_vars(&self) -> HashMap<String, String> {
        self.vars
            .iter()
            .filter(|(_, (_, exp))| *exp)
            .map(|(k, (v, _))| (k.clone(), v.clone()))
            .collect()
    }

    pub fn set_positional(&mut self, params: Vec<String>) {
        self.positional = params;
    }

    pub fn get_positional(&self) -> &[String] {
        &self.positional
    }

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
