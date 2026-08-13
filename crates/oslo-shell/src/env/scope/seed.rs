//! The variables the shell describes *itself* with.
//!
//! Every one is set only if the name is not already taken, so a caller who exported their own
//! `$UID` or `$OSTYPE` keeps it. They are here rather than in `scope.rs` because they answer one
//! question — "what process is this?" — and because that file is at the 600-line limit.

use super::Environment;
use super::environ::environ_set;

impl Environment {
    /// Set the variables that describe the process the shell is running as.
    ///
    /// Unset, these fail *quietly* and wrongly: `[ "$UID" = 0 ]` is the root check in most install
    /// scripts, and an unset `$UID` makes it answer "not root" on a machine where the script is
    /// running as root. `$PPID` names the parent for lock files and process trees.
    ///
    /// Not exported, matching bash: they describe *this* shell, and a child that inherited its
    /// parent's `$PPID` would be told the wrong thing. They are set only if the name is not
    /// already taken, so an inherited environment variable of the same name still wins — the
    /// shell should not overwrite something the caller deliberately put there.
    pub(super) fn seed_process_vars(&mut self) {
        let uid = nix::unistd::getuid().as_raw();
        let euid = nix::unistd::geteuid().as_raw();
        let ppid = nix::unistd::getppid().as_raw();
        // `$SHLVL` counts how deep a shell is nested. Incremented from whatever we inherited, so
        // a shell inside a shell reads 2 — which is what a prompt showing nesting depth needs, and
        // what starship asks for by `--shlvl`.
        let shlvl = std::env::var("SHLVL")
            .ok()
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0)
            .saturating_add(1);
        // Written through to the real environment as well as the table: a child gets its
        // environment from `execve`, so a value that lives only in oslo's map would leave every
        // nested shell reading the *grandparent's* depth.
        let shlvl = shlvl.to_string();
        self.vars.insert("SHLVL".to_string(), (shlvl.clone(), true));
        environ_set("SHLVL", &shlvl);

        // **Which session this is, exported so a child can say so too.** `oslo macros` runs as a
        // child of the shell whose macros it manages, and "off for this session" is a statement
        // about the *parent*: without a name both processes agree on, the manager would write down
        // a session nobody is running.
        //
        // The id itself is decided in `main`, by `track::session::begin`, because *being a shell* is
        // what starts a session and building an `Environment` is not — a tool builds one too, and a
        // tool that stamped a new session over the inherited one would be talking about itself.
        let session = oslo_base::track::session::id();
        self.vars
            .insert("OSLO_SESSION".to_string(), (session.clone(), true));
        environ_set("OSLO_SESSION", &session);
        for (name, value) in [
            ("UID", uid.to_string()),
            ("EUID", euid.to_string()),
            ("PPID", ppid.to_string()),
            // What kind of system this is. Scripts branch on it and nothing set it.
            ("OSTYPE", "linux-gnu".to_string()),
            (
                "MACHTYPE",
                format!("{}-pc-linux-gnu", std::env::consts::ARCH),
            ),
        ] {
            if !self.vars.contains_key(name) {
                self.vars.insert(name.to_string(), (value, false));
            }
        }
    }

    /// The bash version oslo declares itself compatible with.
    ///
    /// **This is a compatibility declaration, not a claim to be bash.** It exists because the
    /// entire shell-integration ecosystem gates on it, and an *absent* `$BASH_VERSINFO` is read as
    /// bash 0.0 — older than any bash that ever shipped. atuin's include guard is
    /// `((BASH_VERSINFO[0] < 3 || ...))`, so with nothing here it concluded the shell predated
    /// 3.1 and skipped its whole integration silently. Not being bash and saying so got oslo
    /// treated worse than a twenty-year-old bash would have been.
    ///
    /// **4.2 rather than 5.x**, deliberately. The number chooses which code path an integration
    /// takes, so claiming more than oslo can execute trades a silent skip for a loud failure
    /// halfway through someone's init script. oslo has what 4.2 implies — `+=`, `[[ =~ ]]`,
    /// indexed arrays, negative subscripts — and does *not* have what 4.4 and 5.x imply:
    /// `${x@P}`, `${x@Q}`, associative arrays, `local -n`. Raising this is a promise, and it
    /// should be raised only as those land.
    const BASH_COMPAT: (u32, u32, u32) = (4, 2, 0);

    /// Declare compatibility with a bash version, the way every integration expects to read it.
    ///
    /// Set only when unset, matching [`Self::seed_process_vars`]: a caller who exported their own
    /// `$BASH_VERSION` meant it, and a shell that overwrote it would be arguing with the person
    /// running it.
    /// `$IFS`, which POSIX says the shell sets at startup to space, tab and newline.
    ///
    /// **It was a default without a variable.** Field splitting read `get_var("IFS")` and fell
    /// back to `" \t\n"` when nothing was there, so splitting behaved correctly while the
    /// *variable* did not exist: `${#IFS}` was 0, `${IFS+SET}` was empty, and under `set -u` any
    /// mention of `$IFS` was a fatal "unbound variable". `/usr/bin/xdg-terminal-exec` opens with
    /// `XTE__OIFS=$IFS` under `set -u` and died there.
    ///
    /// Not exported, matching bash and dash — `export -p` lists it in neither — because a child
    /// process has no use for its parent's field separators.
    ///
    /// Set only when unset, like everything else here: a caller who put `IFS` in the environment
    /// meant it, and POSIX says an inherited value is used.
    pub(super) fn seed_field_separator(&mut self) {
        if !self.vars.contains_key("IFS") {
            self.vars
                .insert("IFS".to_string(), (" \t\n".to_string(), false));
        }
    }

    /// `$PWD`, **exported**, from where the process actually is.
    ///
    /// POSIX: the shell sets `PWD` at startup to the absolute pathname of the current working
    /// directory. oslo only ever wrote it from `cd`, so a shell that had not changed directory yet
    /// had none at all — and nothing that inherits the environment could find out where it was.
    ///
    /// That is not a theoretical gap. `sshd` does not set `PWD`, so a shell logged into over SSH
    /// started with it missing, and every prompt renderer run as a child — hexe, starship,
    /// anything reading `$PWD` because it is handed a pipe rather than a terminal — had no
    /// directory to show until the first `cd` happened to write one.
    ///
    /// An inherited value is kept **only when it names this directory**. POSIX allows using the
    /// inherited one, and a value that survives a `chdir` the shell did not make is a lie: it is
    /// how `$PWD` ends up pointing at where the *parent* was.
    pub(super) fn seed_working_directory(&mut self) {
        let Ok(here) = std::env::current_dir() else {
            return;
        };
        let here = here.to_string_lossy().to_string();
        // Compared by what they resolve to, not as text: an inherited `$PWD` may reach this same
        // directory through a symlink, and POSIX prefers that spelling because it is the one the
        // user typed to get here.
        let inherited_is_here = self
            .vars
            .get("PWD")
            .map(|(value, _)| value.as_str())
            .filter(|value| !value.is_empty())
            .and_then(|value| std::fs::canonicalize(value).ok())
            .is_some_and(|resolved| std::fs::canonicalize(&here).is_ok_and(|now| resolved == now));
        // Written through to the real environment as well as the table, for the reason `SHLVL`
        // above is: a child's environment comes from `execve` reading `environ`, so a value that
        // lives only in oslo's map is invisible to exactly the programs this exists to serve.
        if !inherited_is_here {
            self.vars.insert("PWD".to_string(), (here.clone(), true));
            environ_set("PWD", &here);
            return;
        }
        // Kept, but exported: an inherited-but-unexported `PWD` is invisible to a child, which is
        // the whole reason this exists.
        if let Some(slot) = self.vars.get_mut("PWD") {
            slot.1 = true;
            environ_set("PWD", &slot.0.clone());
        }
    }

    pub(super) fn seed_compatibility_vars(&mut self) {
        let (major, minor, patch) = Self::BASH_COMPAT;
        if !self.vars.contains_key("BASH_VERSION") {
            self.vars.insert(
                "BASH_VERSION".to_string(),
                (format!("{major}.{minor}.{patch}(1)-release"), false),
            );
        }
        if self.get_array("BASH_VERSINFO").is_none() {
            // Six elements, in bash's order: major, minor, patch, build, release status, machine.
            // Scripts index all of them; `BASH_VERSINFO[5]` is what a few use to spot the OS.
            let fields = [
                major.to_string(),
                minor.to_string(),
                patch.to_string(),
                "1".to_string(),
                "release".to_string(),
                std::env::consts::ARCH.to_string() + "-pc-linux-gnu",
            ];
            let mut array = crate::env::scope::array::ShellArray::default();
            for (index, value) in fields.iter().enumerate() {
                array.set(index as i64, value.clone());
            }
            self.set_array("BASH_VERSINFO", array);
        }
    }
}

#[cfg(test)]
mod pwd_tests {
    use crate::env::Environment;

    /// **A shell knows where it is before anything asks.** POSIX requires `PWD` at startup, and
    /// oslo only ever wrote it from `cd` — so a shell logged into over SSH, where `sshd` sets no
    /// `PWD`, had none until the user happened to change directory. Every prompt renderer run as a
    /// child reads `$PWD`, because it is handed a pipe and cannot ask the terminal.
    #[test]
    fn a_new_environment_knows_its_directory() {
        let env = Environment::new();
        let pwd = env.get_var("PWD").expect("PWD is set at startup");
        assert!(!pwd.is_empty());
        assert_eq!(
            std::fs::canonicalize(pwd).ok(),
            std::env::current_dir()
                .ok()
                .and_then(|p| std::fs::canonicalize(p).ok()),
            "PWD must name the directory the process is actually in"
        );
    }
}
