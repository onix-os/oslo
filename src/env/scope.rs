use crate::ast::Command;
use crate::error::Result;
use std::collections::{HashMap, HashSet};
use std::env;
use std::path::PathBuf;

pub type BuiltinFn = fn(&mut Environment, &[String]) -> Result<i32>;

pub struct Environment {
    vars: HashMap<String, (String, bool)>, // (value, is_exported)
    positional: Vec<String>,
    pub last_status: i32,
    pub pid: u32,
    pub last_bg_pid: Option<u32>,
    pub shell_name: String,
    aliases: HashMap<String, String>,
    functions: HashMap<String, Command>,
    custom_builtins: HashMap<String, BuiltinFn>,
    signal_traps: HashMap<String, String>,
    readonly_vars: HashSet<String>,
    dir_stack: Vec<PathBuf>,
    scope_stack: Vec<HashMap<String, Option<(String, bool)>>>,
    /// How many loops are currently executing in this shell.
    ///
    /// `break` and `continue` unwind as errors, which would abandon the rest of the enclosing
    /// command list. Outside any loop they must instead do nothing at all — `break; echo hi`
    /// still prints `hi` — so the builtins consult this before signalling.
    loop_depth: usize,
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
            positional: Vec::new(),
            last_status: 0,
            pid,
            last_bg_pid: None,
            shell_name: "rush".to_string(),
            aliases,
            functions: HashMap::new(),
            custom_builtins: HashMap::new(),
            signal_traps: HashMap::new(),
            readonly_vars: HashSet::new(),
            dir_stack: Vec::new(),
            scope_stack: Vec::new(),
            loop_depth: 0,
        };

        crate::env::builtins::register_default_builtins(&mut env_struct);
        env_struct
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

    pub fn push_scope(&mut self) {
        self.scope_stack.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        if let Some(frame) = self.scope_stack.pop() {
            for (k, orig_val) in frame {
                match orig_val {
                    Some((val, is_exp)) => {
                        self.vars.insert(k.clone(), (val.clone(), is_exp));
                        unsafe {
                            if is_exp {
                                env::set_var(&k, &val);
                            } else {
                                // The variable existed but was not exported. If it was exported
                                // temporarily inside this scope, the process environment still
                                // holds it and would leak into every later child.
                                env::remove_var(&k);
                            }
                        }
                    }
                    None => {
                        self.vars.remove(&k);
                        unsafe {
                            env::remove_var(&k);
                        }
                    }
                }
            }
        }
    }

    /// Remember `name`'s current value in the innermost scope so [`Self::pop_scope`] restores it.
    fn save_for_restore(&mut self, name: &str) {
        if let Some(top_frame) = self.scope_stack.last_mut()
            && !top_frame.contains_key(name)
        {
            top_frame.insert(name.to_string(), self.vars.get(name).cloned());
        }
    }

    pub fn set_local_var(&mut self, name: &str, value: &str) {
        self.save_for_restore(name);
        self.set_var(name, value, false);
    }

    /// Set a variable that is exported for the lifetime of the innermost scope only.
    ///
    /// This is what a command-prefix assignment (`FOO=bar cmd`) needs: `cmd` must see `FOO` in
    /// its environment, and the shell must not.
    pub fn set_local_exported_var(&mut self, name: &str, value: &str) {
        self.save_for_restore(name);
        self.set_var(name, value, true);
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

    pub fn exec_custom_builtin(&mut self, name: &str, args: &[String]) -> Option<Result<i32>> {
        self.custom_builtins
            .get(name)
            .copied()
            .map(|func| func(self, args))
    }

    pub fn get_var(&self, name: &str) -> Option<&str> {
        match name {
            "?" | "$" | "!" | "#" | "*" | "@" => None,
            _ => self.vars.get(name).map(|(v, _)| v.as_str()),
        }
    }

    pub fn get_param(&self, name: &str) -> Option<String> {
        match name {
            "?" => Some(self.last_status.to_string()),
            "$" => Some(self.pid.to_string()),
            "!" => self.last_bg_pid.map(|p| p.to_string()),
            "#" => Some(self.positional.len().to_string()),
            "0" => Some(self.shell_name.clone()),
            "*" | "@" => Some(self.positional.join(" ")),
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

    pub fn set_var(&mut self, name: &str, value: &str, export: bool) {
        if self.is_readonly(name) {
            eprintln!("rush: {}: is read only", name);
            return;
        }
        let is_exp = export || self.vars.get(name).map(|(_, exp)| *exp).unwrap_or(false);
        self.vars
            .insert(name.to_string(), (value.to_string(), is_exp));
        if is_exp {
            unsafe {
                env::set_var(name, value);
            }
        }
    }

    pub fn unset_var(&mut self, name: &str) {
        self.vars.remove(name);
        unsafe {
            env::remove_var(name);
        }
    }

    pub fn export_var(&mut self, name: &str) {
        if let Some((val, exp)) = self.vars.get_mut(name) {
            *exp = true;
            let val_clone = val.clone();
            unsafe {
                env::set_var(name, val_clone);
            }
        } else {
            self.vars.insert(name.to_string(), ("".to_string(), true));
            unsafe {
                env::set_var(name, "");
            }
        }
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

    pub fn register_custom_builtin(&mut self, name: &str, func: BuiltinFn) {
        self.custom_builtins.insert(name.to_string(), func);
    }

    pub fn get_builtin(&self, name: &str) -> Option<&BuiltinFn> {
        self.custom_builtins.get(name)
    }

    /// Every registered builtin name.
    ///
    /// The single source of truth for completion and `type`, so adding a builtin does not
    /// require remembering to update a list somewhere else.
    pub fn builtin_names(&self) -> impl Iterator<Item = &str> {
        self.custom_builtins.keys().map(String::as_str)
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        let trimmed = name.trim();
        matches!(
            trimmed,
            "cd" | "pwd"
                | "echo"
                | "export"
                | "unset"
                | "alias"
                | "unalias"
                | "exit"
                | "break"
                | "continue"
                | "return"
                | "eval"
                | "source"
                | "."
                | "read"
                | "local"
                | "pushd"
                | "popd"
                | "dirs"
                | "readonly"
                | "test"
                | "["
                | "[["
                | "trap"
                | "umask"
                | "wait"
                | "kill"
                | "true"
                | "false"
        ) || self.custom_builtins.contains_key(trimmed)
    }
}
