//! The environment of a Nix dev shell, without entering one — direnv's `use flake` and `use nix`.
//!
//! Here rather than in the Lua API, where it started, because two front doors now want it: an
//! `.envrc` writing `use flake` and a `.env.lua` writing `oslo.direnv.nix_develop()`. One of them
//! having its own copy is how the two would come to disagree about `IGNORED`, and getting that
//! list wrong is silent and severe — see below.
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
//! `cd` into a flake. `IGNORED` is that list, widened to nix's full documented set so a variable
//! that is merely absent from this flake cannot appear in another one and break it.
//!
//! # The rule behind both of the bugs this module has had
//!
//! **`print-dev-env`'s bash output is curated for a shell to consume; `--json` is a raw dump of the
//! builder.** Everything the bash form does *besides* assigning values is invisible to `--json`:
//!
//! * variables it declines to emit at all — `HOME`, `TERM`, `TZ` and two more, handled by
//!   `IGNORED`;
//! * statements it runs after the assignments — the `PATH` restore, handled by `keeping_yours`.
//!
//! Both bugs were the same mistake twice: treating a snapshot of values as if it were the script.
//! A third difference of this kind is likely, so when something loaded from a flake behaves oddly,
//! diff the two forms before looking anywhere else — that is how both of these were found.

use crate::env::Environment;
use std::path::{Path, PathBuf};

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

/// The dev shell's `PATH`, with everything of yours that it does not already have, behind it.
///
/// **This is not our invention — it is a line `--json` cannot carry.** The bash form of
/// `print-dev-env` is bookended:
///
/// ```sh
/// line    3:  nix_saved_PATH="$PATH"                           # save yours
/// line   91:  PATH='/nix/store/…gcc-wrapper/bin:…'             # replace with the dev shell's
/// line 2195:  PATH="$PATH${nix_saved_PATH:+:$nix_saved_PATH}"  # and put yours back, behind it
/// ```
///
/// direnv `eval`s that script, so line 2195 runs and a zsh user keeps `clear` and `git`. `--json`
/// is a snapshot of *values*: line 2195 is a statement, and `nix_saved_PATH` is not among its 144
/// variables at all. So the append has to be done here, or it does not happen.
///
/// Without it, a dev shell's `PATH` is a *build* environment — 36 store entries for this
/// repository's flake, with `ls` and `grep` because coreutils is a build input, and no `clear`, no
/// `git`, and nothing you installed. `cd` into the project and the shell quietly loses half its
/// commands, which is exactly what happened the first time this was used for real.
fn keeping_yours(dev: &str, outer: &str) -> String {
    let mut out: Vec<&str> = dev.split(':').filter(|e| !e.is_empty()).collect();
    for entry in outer.split(':').filter(|e| !e.is_empty()) {
        if !out.contains(&entry) {
            out.push(entry);
        }
    }
    out.join(":")
}

/// The command direnv runs, with the profile that keeps the shell from being garbage-collected.
///
/// `--profile` is not decoration: without it the dev shell's store paths have no GC root, and the
/// next `nix store gc` deletes the toolchain out from under a directory that still points at it.
/// direnv puts the profile under its layout directory and so do we.
///
/// **`args` is passed through verbatim, and nothing here reads it.** direnv's `use_flake` ends in
/// `nix print-dev-env --profile "$profile" "$@"`, which is what makes
/// `use flake --option warn-dirty false` work: the flags are nix's, not direnv's. Taking the first
/// argument as the installable instead turned that line into `print-dev-env … '--option'` and nix
/// answered "flag '--option' requires 2 argument(s), but only 0 were given" — a message about a
/// flag the `.envrc` never asked to be alone.
///
/// An empty list is left empty rather than defaulting to `.`: `print-dev-env` already resolves the
/// current directory's flake, which is how a bare `use flake` works in direnv too.
pub fn command(args: &[String], profile: &str) -> String {
    let mut out = format!(
        "nix --extra-experimental-features 'nix-command flakes' print-dev-env --json --profile {}",
        shell_quote(profile),
    );
    for arg in args {
        out.push(' ');
        out.push_str(&shell_quote(arg));
    }
    out
}

fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// The directory the project's `.direnv` belongs in.
///
/// **The rc file's own, not the shell's.** These paths used to be relative to the working
/// directory, so arriving in a subdirectory of a project scattered a `.direnv` into whichever
/// directory the shell happened to be standing in — and the profile written there was a different
/// GC root each time.
fn root() -> PathBuf {
    let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    super::find::applicable(&here)
        .and_then(|rc| super::find::owner(&rc))
        .unwrap_or(here)
}

/// The profile path, created if it is not there, under the project's `.direnv`.
pub fn profile() -> String {
    let profile = root().join(".direnv/flake-profile");
    if let Some(parent) = profile.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    profile.to_string_lossy().into_owned()
}

/// The files that decide what the dev shell contains.
const INPUTS: &[&str] = &["flake.nix", "flake.lock", "shell.nix", "default.nix"];

/// What the cached answer was computed from: the arguments, and every input as it stood.
///
/// Length and mtime, the same pair the rc files are stamped with and for the same reason — mtime
/// alone has one-second granularity on some filesystems.
fn key(root: &Path, args: &[String]) -> String {
    let mut key = format!("1 {}", args.join(" "));
    for name in INPUTS {
        let stamp = std::fs::metadata(root.join(name))
            .ok()
            .map(|meta| {
                let when = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                format!("{}:{when}", meta.len())
            })
            .unwrap_or_else(|| "-".to_string());
        key.push(' ');
        key.push_str(&stamp);
    }
    key
}

/// Where the evaluated environment is kept between runs.
fn cache(root: &Path) -> PathBuf {
    root.join(".direnv/dev-env.json")
}

/// **The evaluation, remembered.** `nix print-dev-env` costs about half a second on a warm store
/// and several on a cold one, and it is asked the same question every time: this project's flake
/// has not moved since the last arrival. That is once per `cd` into the project and once per new
/// shell — a new pane, a nested `oslo` — which is often enough to be the slowest thing the shell
/// does all day.
///
/// Keyed on the inputs rather than timed out, so editing `flake.nix` re-evaluates immediately and
/// nothing else does. `direnv reload` drops it outright, which is the escape hatch for the case
/// this cannot see: nix itself, or something the flake reads that is not one of [`INPUTS`].
fn cached(args: &[String]) -> Option<String> {
    let root = root();
    let text = std::fs::read_to_string(cache(&root)).ok()?;
    let (head, body) = text.split_once('\n')?;
    (head == key(&root, args)).then(|| body.to_string())
}

fn remember(args: &[String], json: &str) {
    let root = root();
    let path = cache(&root);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::write(&path, format!("{}\n{json}", key(&root, args))).is_err() {
        return;
    }
    // **Owner-only.** This is a verbatim dump of a dev shell's environment, which for a good many
    // projects means tokens and connection strings. It sits inside the working tree, where the
    // umask would otherwise have left it world-readable on a shared machine.
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
}

/// Drop the remembered evaluation, for `direnv reload`.
pub fn forget() {
    let _ = std::fs::remove_file(cache(&root()));
}

/// Run `nix print-dev-env` with `args` and apply what it exports. Answers how many.
///
/// The one implementation, called by `use flake` from an `.envrc` and by
/// `oslo.direnv.nix_develop()` from a `.env.lua`.
pub fn apply(env: &mut Environment, args: &[String]) -> Result<usize, String> {
    apply_with(env, args, false)
}

/// The same, and then `shellHook` if `hook` asks for it.
///
/// **Off unless asked, because it is somebody else's script.** The variables a flake exports are
/// data; `shellHook` is a bash program that runs on every entry to the directory — it can print,
/// write files, start things. `nix develop` and nix-direnv run it and plain direnv does not; oslo
/// sides with plain direnv by default and lets a project opt in, so that `cd` into a clone of
/// somebody else's repository is not an invitation.
pub fn apply_with(env: &mut Environment, args: &[String], hook: bool) -> Result<usize, String> {
    let json = match cached(args) {
        Some(remembered) => remembered,
        None => {
            let fresh = crate::exec::eval_command_substitution(env, &command(args, &profile()))
                .map_err(|e| e.to_string())?;
            if fresh.trim().is_empty() {
                return Err(
                    "`nix print-dev-env` produced nothing — is nix installed, and does this \
                            directory have a flake?"
                        .to_string(),
                );
            }
            // Written only after it parses, so a truncated or error-shaped answer is not the thing
            // every later arrival is served from.
            exported_from(&fresh)?;
            remember(args, &fresh);
            fresh
        }
    };
    let exported = exported_from(&json)?;
    let count = exported.len();
    let outer_path = env.get_var("PATH").unwrap_or_default().to_string();
    let mut script = None;
    for (name, value) in exported {
        if name == "PATH" {
            env.set_var(&name, &keeping_yours(&value, &outer_path), true);
            continue;
        }
        if hook && name == "shellHook" {
            script = Some(value.clone());
        }
        env.set_var(&name, &value, true);
    }
    // **After the variables, not before.** The hook is written expecting the shell it is entering:
    // the flake's own `$MESHTASTIC_PROTO_DIR` and the rest have to be set for it to have anything
    // to say. Run through oslo rather than bash — a `.env.lua` project has already chosen this
    // shell, and shelling out would make the hook's `$PATH` a different one from the caller's.
    if let Some(script) = script.filter(|s| !s.trim().is_empty())
        && let Err(err) =
            crate::env::builtins::builtin_eval(env, &["eval".to_string(), "--".to_string(), script])
    {
        return Err(format!("shellHook: {err}"));
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_builders_home_never_escapes() {
        let json = r#"{"variables":{"HOME":{"type":"exported","value":"/homeless-shelter"},
                       "CC":{"type":"exported","value":"gcc"}}}"#;
        let got = exported_from(json).expect("parsed");
        assert_eq!(got, vec![("CC".to_string(), "gcc".to_string())]);
    }

    #[test]
    fn the_whole_ignore_list_is_dropped() {
        for name in IGNORED {
            let json = format!(r#"{{"variables":{{"{name}":{{"type":"exported","value":"x"}}}}}}"#);
            assert!(
                exported_from(&json).expect("parsed").is_empty(),
                "{name} must not escape the dev shell"
            );
        }
    }

    #[test]
    fn only_exported_variables_are_taken() {
        let json = r#"{"variables":{"A":{"type":"var","value":"1"},
                       "B":{"type":"array","value":["x"]},
                       "C":{"type":"exported","value":"3"}}}"#;
        let got = exported_from(json).expect("parsed");
        assert_eq!(got, vec![("C".to_string(), "3".to_string())]);
    }

    #[test]
    fn exported_bash_functions_are_dropped() {
        let json = r#"{"variables":{"BASH_FUNC_foo%%":{"type":"exported","value":"() { :; }"}}}"#;
        assert!(exported_from(json).expect("parsed").is_empty());
    }

    #[test]
    fn the_command_names_a_profile_and_quotes_its_arguments() {
        let got = command(&[String::from("..#dev shell")], ".direnv/flake-profile");
        assert!(got.contains("--profile '.direnv/flake-profile'"));
        assert!(got.ends_with("'..#dev shell'"));
    }

    #[test]
    fn your_own_path_survives_behind_the_dev_shells() {
        let got = keeping_yours("/nix/a:/nix/b", "/usr/bin:/nix/a:/home/me/bin");
        assert_eq!(got, "/nix/a:/nix/b:/usr/bin:/home/me/bin");
    }

    #[test]
    fn an_entry_in_both_appears_once() {
        assert_eq!(keeping_yours("/x", "/x"), "/x");
    }

    /// **An edited flake re-evaluates, and an untouched one does not.**
    ///
    /// The whole value of the cache is that it is keyed rather than timed, and the whole risk is
    /// serving a dev shell that no longer matches the flake that describes it. The root is a
    /// parameter precisely so this can be asked without moving the process's working directory,
    /// which every other test running beside it shares.
    #[test]
    fn the_key_moves_when_an_input_does_and_not_otherwise() {
        let project = tempfile::tempdir().expect("temp dir");
        let root = project.path();
        std::fs::write(root.join("flake.nix"), "{ }").expect("write");

        let first = key(root, &[String::from(".")]);
        assert_eq!(
            first,
            key(root, &[String::from(".")]),
            "nothing changed, nothing to re-do"
        );
        assert_ne!(
            first,
            key(root, &[String::from("..#other")]),
            "another shell, another key"
        );

        std::fs::write(root.join("flake.nix"), "{ inputs = {}; }").expect("write");
        assert_ne!(
            first,
            key(root, &[String::from(".")]),
            "an edited flake must re-evaluate"
        );
    }

    /// A project with no flake at all still has a stable key rather than a changing one.
    #[test]
    fn a_project_with_no_inputs_is_still_answerable() {
        let project = tempfile::tempdir().expect("temp dir");
        assert_eq!(
            key(project.path(), &[String::from(".")]),
            key(project.path(), &[String::from(".")])
        );
    }

    #[test]
    fn output_that_is_not_print_dev_env_is_refused_by_name() {
        let problem = exported_from(r#"{"nope":1}"#).expect_err("refused");
        assert!(problem.contains("print-dev-env"), "{problem}");
    }

    /// `shellHook` is exported like anything else, so it lands in the environment either way. What
    /// the flag decides is whether it is also *run*.
    #[test]
    fn the_hook_is_a_variable_before_it_is_a_script() {
        let json = r#"{"variables":{"shellHook":{"type":"exported","value":"echo hi"},
                       "CC":{"type":"exported","value":"gcc"}}}"#;
        let got = exported_from(json).expect("parsed");
        assert!(
            got.contains(&("shellHook".to_string(), "echo hi".to_string())),
            "the hook is still imported as a variable: {got:?}"
        );
    }
}

#[cfg(test)]
mod command_tests {
    use super::command;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_string()).collect()
    }

    /// `use flake --option warn-dirty false` is a real line in a real `.envrc`, and every word
    /// after `use flake` belongs to `nix`. Reading the first one as the installable sent nix a
    /// lone `--option`, which it rejects for wanting two arguments it was never given.
    #[test]
    fn every_argument_reaches_nix_in_order() {
        let built = command(
            &args(&["--option", "warn-dirty", "false"]),
            "/p/flake-profile",
        );
        assert!(
            built.ends_with("'--option' 'warn-dirty' 'false'"),
            "{built}"
        );
        assert!(built.contains("--profile '/p/flake-profile'"), "{built}");
    }

    /// A bare `use flake` names nothing, and `print-dev-env` resolves this directory itself —
    /// which is what direnv relies on, its `"$@"` being empty in exactly this case.
    #[test]
    fn no_arguments_means_no_installable_rather_than_a_dot() {
        let built = command(&[], "/p/flake-profile");
        assert!(built.ends_with("--profile '/p/flake-profile'"), "{built}");
    }

    /// A named installable still arrives, and quoting survives a word that would otherwise split.
    #[test]
    fn an_installable_and_an_awkward_word_are_quoted() {
        let built = command(&args(&[".#other", "a b"]), "/p");
        assert!(built.ends_with("'.#other' 'a b'"), "{built}");
    }
}
