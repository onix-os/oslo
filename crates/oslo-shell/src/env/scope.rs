mod array;
mod environ;
mod frames;
mod names;
mod options;
mod record;
mod registry;
mod seed;
#[cfg(test)]
mod tests;
mod traps;

use crate::env::nesting::{DepthGuard, MAX_FUNCTION_DEPTH, MAX_SCRIPT_DEPTH};
use crate::env::options::ShellOptions;
use environ::{environ_remove, environ_set, reject_unrepresentable};
use frames::ScopeFrame;
use oslo_base::ast::Command;
use oslo_base::error::Result;
use registry::BuiltinRegistry;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;

pub use array::{ShellArray, array_literal_body};
pub use registry::{BuiltinFn, is_special_builtin};

/// One running process substitution: the descriptor the caller was given, and the child feeding it.
///
/// **Declared here, beside the list that holds it, rather than in `exec::procsub` where it is
/// opened and closed.** The store is the bottom of the shell's dependency graph — nothing it holds
/// may point back up at the executor — and this was the single field that did. Two OS handles is
/// all it is; the machinery that fills them in stays with the code that forks.
pub struct Substitution {
    pub fd: std::os::fd::RawFd,
    pub child: nix::unistd::Pid,
}

/// What a call frame entered without a name reads as, in `caller`'s output.
///
/// bash's own spelling for the same gap: `f() { caller; }; f` in `bash -c` prints `1 NULL`,
/// because there is no file for the frame to have come from.
pub const UNNAMED_FUNCTION: &str = "NULL";

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
    /// What `$LINENO` was last set to by [`Self::note_line`], or 0 when something else touched it.
    published_line: u32,
    aliases: HashMap<String, String>,
    /// Stored variables that have not been asked for yet: name to the *recipe* for its value.
    ///
    /// `oslo macros add --var GITHUB_TOKEN '$(oslo secret get gh-token)'` puts a line here, not a
    /// token. The first expansion of `$GITHUB_TOKEN` runs it, exports the result, and takes the
    /// entry out — so the cost of a variable that decrypts something is paid by the command that
    /// needed it and by no other, and a shell that never mentions the name never runs it at all.
    lazy: HashMap<String, String>,
    functions: HashMap<String, Arc<Command>>,
    /// Every builtin this shell has. The one list consulted by [`Self::is_builtin`], the
    /// dispatcher in `exec::simple` and `type`; see the `registry` submodule.
    builtins: BuiltinRegistry,
    /// Process substitutions opened while expanding the command now being prepared.
    ///
    /// Lives here because the descriptor has to outlive *expansion* — the program opens
    /// `/dev/fd/N` only once every word is expanded and the command runs — but must not outlive
    /// the command. `crate::exec::simple` closes them when it is done.
    pub procsubs: Vec<Substitution>,
    signal_traps: HashMap<String, String>,
    /// Traps this shell inherited and then reset because it is a subshell.
    ///
    /// POSIX resets a subshell's traps to their default *action*, but carves out command
    /// substitution so that `saved=$(trap)` — the save-and-restore idiom — still reports what the
    /// parent had. Keeping the strings here separates the two: nothing ever *runs* from this map,
    /// and [`Self::listable_traps`] is the only thing that reads it.
    inherited_traps: HashMap<String, String>,
    readonly_vars: HashSet<String>,
    /// Names `export` was told about before they existed.
    ///
    /// `export V` with no value marks `V` for export and leaves it **unset**: bash gives an empty
    /// `${V+set}` and no `V=` in `env`, and a later `V=1` is exported. Creating it empty instead
    /// made it answer "set" to every test, and put a spurious `V=` in every child's environment.
    /// [`Self::set_var`] spends the intention when the name is finally assigned.
    pending_exports: HashSet<String>,
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
    /// The name of every shell function on that chain, outermost first. Pushed and popped with
    /// the counter above; read by `caller`.
    call_stack: Vec<String>,
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

        // **No aliases here.** See [`Environment::seed_interactive_aliases`]: the three
        // conveniences oslo ships belong to a person at a prompt, and this constructor is also
        // every script, every `sh -c` and every subshell.
        let aliases = HashMap::new();

        let mut env_struct = Self {
            vars,
            arrays: HashMap::new(),
            positional: Vec::new(),
            last_status: 0,
            pid,
            last_bg_pid: None,
            shell_name: "oslo".to_string(),
            current_pid: pid,
            pipeline_status: vec![0],
            substitution_status: None,
            published_line: 0,
            aliases,
            lazy: HashMap::new(),
            functions: HashMap::new(),
            builtins: BuiltinRegistry::default(),
            procsubs: Vec::new(),
            signal_traps: HashMap::new(),
            inherited_traps: HashMap::new(),
            readonly_vars: HashSet::new(),
            pending_exports: HashSet::new(),
            dir_stack: Vec::new(),
            scope_stack: Vec::new(),
            loop_depth: 0,
            function_depth: DepthGuard::new(MAX_FUNCTION_DEPTH),
            call_stack: Vec::new(),
            script_depth: DepthGuard::new(MAX_SCRIPT_DEPTH),
            options: ShellOptions::default(),
        };

        crate::env::dynamic::start();
        crate::env::builtins::register_default_builtins(&mut env_struct);
        env_struct.seed_process_vars();
        env_struct.seed_field_separator();
        env_struct.seed_working_directory();
        env_struct.seed_option_index();
        env_struct.seed_compatibility_vars();
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
        // Moved rather than dropped: the actions stop applying, but `$(trap)` still has to be
        // able to report them. See [`Self::inherited_traps`].
        self.inherited_traps = std::mem::take(&mut self.signal_traps);
    }

    /// Whether this environment belongs to a forked subshell rather than the top-level shell.
    pub fn in_subshell(&self) -> bool {
        self.current_pid != self.pid
    }

    /// This process's real pid, as opposed to `$$`. See [`Self::enter_subshell`].
    pub fn current_pid(&self) -> u32 {
        self.current_pid
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
    /// including an unwinding `return` or error. A refused entry is not entered and must not be
    /// exited.
    ///
    /// Prefer [`Self::enter_function_named`]: `caller` can only report a name that was recorded
    /// on the way in, and this form records the placeholder bash prints when it has none.
    pub fn enter_function(&mut self) -> Result<()> {
        self.enter_function_named(UNNAMED_FUNCTION)
    }

    /// Begin a shell-function call, recording which function it is.
    ///
    /// The name is what `caller n` reports as the second field. Kept beside the depth counter
    /// rather than in a table of its own so the two cannot drift: one push, one pop, both here.
    pub fn enter_function_named(&mut self, name: &str) -> Result<()> {
        self.function_depth.enter()?;
        self.call_stack.push(name.to_string());
        Ok(())
    }

    pub fn exit_function(&mut self) {
        self.function_depth.exit();
        self.call_stack.pop();
    }

    /// The shell functions currently executing, innermost last.
    ///
    /// A frame entered through [`Self::enter_function`] rather than
    /// [`Self::enter_function_named`] reads as [`UNNAMED_FUNCTION`].
    pub fn call_stack(&self) -> &[String] {
        &self.call_stack
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
                // A set variable always wins: `SECONDS=0` is an idiom, `RANDOM=42` asks for a
                // reproducible sequence, and an exported `EPOCHSECONDS` from a parent must not be
                // shadowed by our clock.
                self.get_var(name)
                    .map(|s| s.to_string())
                    .or_else(|| crate::env::dynamic::value(name))
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
        // An assignment to `LINENO` from a script is the shell's cue that its own record of what
        // it last published is no longer the truth. See [`Self::note_line`].
        if name == "LINENO" {
            self.published_line = 0;
        }
        if self.is_readonly(name) {
            eprintln!("oslo: {}: is read only", name);
            return false;
        }
        if reject_unrepresentable(name, value) {
            return false;
        }
        if let Some(array) = self.arrays.get_mut(name) {
            array.set(0, value);
            return true;
        }
        // `set -a` exports every name that is assigned to, which is why this belongs here and not
        // at the `name=value` site: POSIX applies it to *any* assignment, so `read`, a `for` loop
        // variable and `${v:=x}` are all covered by deciding it once. The idiom it exists for is
        // `set -a; . /etc/os-release`, which before this exported nothing at all.
        // A pending `export V` is honoured and spent here; the variable carries the flag now.
        let was_pending = self.pending_exports.remove(name);
        let is_exp = export
            || was_pending
            || self.allexport()
            || self.vars.get(name).map(|(_, exp)| *exp).unwrap_or(false);
        self.vars
            .insert(name.to_string(), (value.to_string(), is_exp));
        if is_exp {
            environ_set(name, value);
        }
        if name == "PATH" {
            // Every remembered command location was resolved through the old `PATH`, so keeping
            // them would make `PATH=/new/bin:$PATH; tool` still run the old `tool` — the one
            // failure a command cache is always blamed for. bash flushes here too.
            crate::env::builtins::hash_forget_all();
        }
        true
    }

    /// Remove `name` entirely, in whichever shape it has. `unset a` drops the whole array, not
    /// just its element 0.
    /// Every exported name and its value — what a child process would be handed.
    ///
    /// Exported only, because that is the question a caller is asking when it wants "the
    /// environment": a shell-local variable is not part of it and would mislead anyone iterating
    /// this to decide what a command will see.
    /// Every variable and whether it is exported, for a caller that must put the shell back exactly
    /// as it found it.
    ///
    /// [`Self::exported_vars`] is the right answer to "what will a child see" and the wrong one to
    /// "what was here before": a shell-local variable that a directory environment then exports has
    /// to come back *local*, and a snapshot of the exported set alone cannot say that — it would
    /// remove the variable entirely on the way out.
    pub fn all_vars(&self) -> Vec<(String, String, bool)> {
        let mut out: Vec<(String, String, bool)> = self
            .vars
            .iter()
            .map(|(name, (value, exported))| (name.clone(), value.clone(), *exported))
            .collect();
        out.sort();
        out
    }

    pub fn exported_vars(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .vars
            .iter()
            .filter(|(_, (_, exported))| *exported)
            .map(|(name, (value, _))| (name.clone(), value.clone()))
            .collect();
        // Sorted so a caller that prints them gets a stable order; a HashMap's is arbitrary and
        // would make any test or diff over the result flap.
        out.sort();
        out
    }

    pub fn unset_var(&mut self, name: &str) {
        if name == "LINENO" {
            self.published_line = 0;
        }
        self.vars.remove(name);
        self.arrays.remove(name);
        // `unset` undoes a pending `export` too, or `export V; unset V; V=1` would still export.
        self.pending_exports.remove(name);
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
            // Recorded, not created. See `pending_exports`.
            self.pending_exports.insert(name.to_string());
        }
        true
    }

    pub fn get_all_vars(&self) -> HashMap<String, String> {
        self.vars
            .iter()
            .map(|(k, (v, _))| (k.clone(), v.clone()))
            .collect()
    }

    /// Whether `name` would be handed to a child process.
    ///
    /// One lookup rather than building the whole exported map, which is what a caller that only
    /// wants to answer this about a single name was otherwise reduced to.
    pub fn is_exported(&self, name: &str) -> bool {
        self.vars.get(name).is_some_and(|(_, exported)| *exported)
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

    /// Install `params` as `$1…` and hand back what was there.
    ///
    /// What a function call wants. Reading the caller's parameters out and putting them back
    /// afterwards copied every one of them twice per call; both halves are moves here.
    pub fn swap_positional(&mut self, params: Vec<String>) -> Vec<String> {
        std::mem::replace(&mut self.positional, params)
    }
}

/// The editor's view of the shell.
///
/// **The store implements the interface rather than the editor holding the store.** Completion and
/// highlighting need to ask eight questions — what is `$PS2`, what is on `$PATH`, which names are
/// builtins, aliases or functions — and holding an `Environment` to ask them pointed the interface
/// layer at the shell, which is built on top of it. See [`oslo_ui::shell::Shell`].
impl oslo_ui::shell::Shell for Environment {
    fn interactive(&self) -> bool {
        Environment::interactive(self)
    }

    fn var(&self, name: &str) -> Option<&str> {
        self.get_var(name)
    }

    fn vars(&self) -> HashMap<String, String> {
        self.get_all_vars()
    }

    fn is_builtin(&self, name: &str) -> bool {
        Environment::is_builtin(self, name)
    }

    fn builtin_names(&self) -> Vec<String> {
        Environment::builtin_names(self)
            .map(str::to_string)
            .collect()
    }

    fn alias(&self, name: &str) -> Option<&str> {
        self.get_alias(name)
    }

    fn aliases(&self) -> &HashMap<String, String> {
        self.get_aliases()
    }

    fn is_function(&self, name: &str) -> bool {
        self.get_function(name).is_some()
    }

    fn functions(&self) -> &HashMap<String, Arc<Command>> {
        self.get_functions()
    }
}
