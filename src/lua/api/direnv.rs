//! `oslo.direnv` — the library a `.env.lua` is written against.
//!
//! A namespace rather than more functions on `oslo`, because these are for one job. `oslo` itself
//! had grown twenty-four flat entries with `nix_develop` sitting between `login` and `path_add`,
//! which tells a reader nothing about which of them belong together. `fs`, `json`, `path`, `proc`
//! and `re` were already tables; this is the same idea applied to the directory environment.
//!
//! # `oslo.direnv.nix_develop` — the environment of a Nix dev shell, without entering one.
//!
//! direnv's `use flake` is four lines of shell (`stdlib.sh`):
//!
//! ```sh
//! watch_file flake.nix; watch_file flake.lock
//! eval "$(nix print-dev-env --profile "$(direnv_layout_dir)/flake-profile" "$@")"
//! nix profile wipe-history --profile "$(direnv_layout_dir)/flake-profile"
//! ```
//!
//! The `eval` is the whole trick: `nix print-dev-env` emits **bash** that sets the environment, and
//! direnv hands it to the bash it is already running inside. oslo will not do that — evaluating a
//! hundred kilobytes of generated bash on every arrival is exactly the sort of thing a directory
//! environment must not do — so this uses `--json`, which is the same information as data.
//!
//! # The trap in `--json`, which cost an afternoon to find
//!
//! **The two forms do not contain the same variables.** Deriving the difference on nix 2.34 against
//! this repository's own flake:
//!
//! ```text
//! in --json but never emitted as bash:  HOME NIX_ENFORCE_PURITY NIX_LOG_FD TERM TZ
//! ```
//!
//! `nix` filters those out of the shell form because setting them would wreck the shell you are
//! standing in — `HOME` inside a derivation is `/homeless-shelter`. The JSON form applies no such
//! filter; it is a faithful dump of the builder's environment. So anything built on `--json` has to
//! reproduce that list, and a version that does not will silently repoint `$HOME` the moment you
//! `cd` into a flake. [`IGNORED`] is that list, widened to nix's full documented set so a variable
//! that is merely absent from this flake cannot appear in another one and break it.
//!
//! # Why this is in Rust when nothing else is
//!
//! It could be written in Lua — `oslo.proc.capture`, `oslo.json` and `oslo.env.set` are all there, and
//! about forty lines would do it. It is here because of the paragraph above: the failure mode is
//! severe, silent, and not something you would think to test. A recipe everyone copies is a list
//! everyone copies wrong.

use super::util::{put, text};
use crate::env::Environment;
use crate::lua::eval::{LuaError, Table, Value};
use std::sync::{Arc, Mutex};

/// Variables that must never be taken from a dev shell into the shell you are using.
///
/// The first five are what nix itself withholds from the bash form. The rest are the remainder of
/// nix's own `ignoreVars` (`src/nix/develop.cc`) — absent from this flake, but present in others,
/// and each one would be a different flavour of broken: `PWD` and `OLDPWD` would lie about where
/// you are, `SHELL` would point at the store's bash, `TMPDIR` at a build directory that no longer
/// exists, `SHLVL` would corrupt the nesting count.
const IGNORED: &[&str] = &[
    "BASHOPTS",
    "EUID",
    "HOME",
    "HOSTNAME",
    "NIX_BUILD_TOP",
    "NIX_ENFORCE_PURITY",
    "NIX_LOG_FD",
    "NIX_REMOTE",
    "OLDPWD",
    "PPID",
    "PWD",
    "SHELL",
    "SHELLOPTS",
    "SHLVL",
    "SSL_CERT_FILE",
    "TEMP",
    "TEMPDIR",
    "TERM",
    "TMP",
    "TMPDIR",
    "TZ",
    "UID",
    "_",
];

/// Whether this variable may be carried out of the dev shell.
fn wanted(name: &str) -> bool {
    !IGNORED.contains(&name)
        // `BASH_FUNC_x%%` and friends are exported bash functions. oslo's functions are not bash's,
        // and importing the encoding would put unrunnable text in the environment of every child.
        && !name.starts_with("BASH_FUNC_")
}

/// The exported variables of a dev shell, from `nix print-dev-env --json` output.
///
/// Only `type == "exported"`. A `var` is shell-local to the builder — `SHELL` arrives that way —
/// and an `array` is a bash array, which is a shape a POSIX environment cannot hold. Both are
/// dropped rather than flattened into something that looks like a value and is not.
pub fn exported_from(json: &str) -> Result<Vec<(String, String)>, String> {
    let parsed: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let Some(variables) = parsed.get("variables").and_then(|v| v.as_object()) else {
        return Err(
            "no `variables` in the output; is this `nix print-dev-env --json`?".to_string(),
        );
    };
    let mut out = Vec::new();
    for (name, entry) in variables {
        if !wanted(name) {
            continue;
        }
        if entry.get("type").and_then(|t| t.as_str()) != Some("exported") {
            continue;
        }
        if let Some(value) = entry.get("value").and_then(|v| v.as_str()) {
            out.push((name.clone(), value.to_string()));
        }
    }
    out.sort();
    Ok(out)
}

/// The command direnv runs, with the profile that keeps the shell from being garbage-collected.
///
/// `--profile` is not decoration: without it the dev shell's store paths have no GC root, and the
/// next `nix store gc` deletes the toolchain out from under a directory that still points at it.
/// direnv puts the profile under its layout directory and so do we.
fn command(installable: &str, profile: &str) -> String {
    format!(
        "nix --extra-experimental-features 'nix-command flakes' print-dev-env --json \
         --profile {} {}",
        shell_quote(profile),
        shell_quote(installable)
    )
}

fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// Build the `oslo.direnv` table.
pub fn build(env: &Arc<Mutex<Environment>>) -> Value {
    let mut it = Table::new();
    nix_develop(&mut it, env);
    path_add(&mut it, env);
    Value::table(it)
}

fn path_add(it: &mut Table, env: &Arc<Mutex<Environment>>) {
    // oslo.direnv.path_add(dir, [var]) — put `dir` on the front of `$PATH`, or of the named variable.
    //
    // The single most common thing a `.env.lua` does, and spelling it by hand is both wordy and
    // easy to get subtly wrong: forgetting the separator, or appending rather than prepending so
    // the project's own tool loses to the system one. Relative paths resolve against the current
    // directory, because a directory environment saying `./bin` means *its* bin.
    //
    // Idempotent: a directory already on the front is not added twice, so a reload does not grow
    // the variable each time.
    let env = Arc::clone(env);
    put(it, "path_add", move |_, args| {
        let dir = text(&args, 1, "oslo.direnv.path_add")?;
        let name = match args.get(1) {
            Some(Value::Str(name)) => name.to_string(),
            _ => "PATH".to_string(),
        };
        let dir: String = dir.to_string();
        let joined = match std::path::Path::new(&dir).is_absolute() {
            true => std::path::PathBuf::from(&dir),
            false => std::env::current_dir().unwrap_or_default().join(&dir),
        };
        // Normalised lexically, not with `canonicalize`: the directory may not exist yet (a build
        // tree that has not been made), and canonicalising would fail there and also resolve
        // symlinks the user wrote deliberately. `components()` drops the `.` in `./bin`, which is
        // what makes `path_add("bin")` and `path_add("./bin")` the same entry — without it the
        // idempotence check below compares two spellings of one directory and adds it twice.
        let absolute = joined
            .components()
            .collect::<std::path::PathBuf>()
            .to_string_lossy()
            .to_string();
        let mut guard = crate::lua::engine::borrow_env(&env)?;
        let current = guard.get_var(&name).unwrap_or_default().to_string();
        if current.split(':').any(|entry| entry == absolute) {
            return Ok(vec![Value::str(current)]);
        }
        let joined = match current.is_empty() {
            true => absolute,
            false => format!("{absolute}:{current}"),
        };
        guard.set_var(&name, &joined, true);
        Ok(vec![Value::str(joined)])
    });
}

fn nix_develop(it: &mut Table, env: &Arc<Mutex<Environment>>) {
    let env = Arc::clone(env);
    put(it, "nix_develop", move |_, args| {
        // `oslo.direnv.nix_develop()` means this directory's flake; a string names another installable,
        // exactly as `use flake ..#other` does.
        let installable = match args.first() {
            Some(Value::Str(_)) => text(&args, 1, "oslo.direnv.nix_develop")?.to_string(),
            _ => ".".to_string(),
        };
        let profile = ".direnv/flake-profile".to_string();
        if let Some(parent) = std::path::Path::new(&profile).parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let json = {
            let mut guard = crate::lua::engine::borrow_env(&env)?;
            crate::exec::eval_command_substitution(&mut guard, &command(&installable, &profile))
                .map_err(|e| LuaError::new(format!("oslo.direnv.nix_develop: {e}")))?
        };
        if json.trim().is_empty() {
            return Err(LuaError::new(
                "oslo.direnv.nix_develop: `nix print-dev-env` produced nothing — is nix installed, and \
                 does this directory have a flake?",
            ));
        }

        let exported = exported_from(&json).map_err(LuaError::new)?;
        let count = exported.len();
        {
            let mut guard = crate::lua::engine::borrow_env(&env)?;
            for (name, value) in exported {
                guard.set_var(&name, &value, true);
            }
        }
        Ok(vec![Value::Number(crate::lua::eval::Number::Int(
            count as i64,
        ))])
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The variable that makes this worth writing in Rust.
    ///
    /// `HOME` is `/homeless-shelter` inside a derivation. `nix print-dev-env` withholds it from the
    /// bash form; `--json` does not. Taking it would repoint `$HOME` for the whole session the
    /// moment somebody `cd`s into a flake, and nothing about the symptom would point back here.
    #[test]
    fn the_builders_home_never_escapes() {
        let json = r#"{"variables":{
            "HOME":{"type":"exported","value":"/homeless-shelter"},
            "CC":{"type":"exported","value":"/nix/store/x/bin/gcc"}
        }}"#;
        let got = exported_from(json).expect("parses");
        assert_eq!(
            got,
            vec![("CC".to_string(), "/nix/store/x/bin/gcc".to_string())]
        );
    }

    /// Every name nix withholds from the shell form, checked as a set rather than one by one.
    #[test]
    fn the_whole_ignore_list_is_dropped() {
        let entries: Vec<String> = IGNORED
            .iter()
            .map(|name| format!(r#""{name}":{{"type":"exported","value":"x"}}"#))
            .collect();
        let json = format!(r#"{{"variables":{{{}}}}}"#, entries.join(","));
        assert!(exported_from(&json).expect("parses").is_empty());
    }

    /// Only `exported`. A `var` is builder-local and an `array` has no POSIX shape.
    #[test]
    fn only_exported_variables_are_taken() {
        let json = r#"{"variables":{
            "KEPT":{"type":"exported","value":"yes"},
            "LOCAL":{"type":"var","value":"no"},
            "LIST":{"type":"array","value":["a","b"]}
        }}"#;
        let got = exported_from(json).expect("parses");
        assert_eq!(got, vec![("KEPT".to_string(), "yes".to_string())]);
    }

    /// An exported bash function is not a value oslo can carry.
    #[test]
    fn exported_bash_functions_are_dropped() {
        let json = r#"{"variables":{
            "BASH_FUNC_genericBuild%%":{"type":"exported","value":"() { :; }"}
        }}"#;
        assert!(exported_from(json).expect("parses").is_empty());
    }

    /// A profile path is what stops the toolchain being garbage-collected out from under you.
    #[test]
    fn the_command_names_a_profile_and_quotes_its_arguments() {
        let built = command(".", ".direnv/flake-profile");
        assert!(
            built.contains("--profile '.direnv/flake-profile'"),
            "{built}"
        );
        assert!(built.contains("print-dev-env --json"), "{built}");
        // An installable with a quote in it must not end the quoting.
        assert!(!command("it's", ".direnv/p").contains("'it's'"));
    }

    #[test]
    fn output_that_is_not_print_dev_env_is_refused_by_name() {
        let problem = exported_from(r#"{"something":{}}"#).expect_err("must refuse");
        assert!(problem.contains("print-dev-env"), "{problem}");
    }
}
