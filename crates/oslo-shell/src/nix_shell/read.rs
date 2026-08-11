//! Reading `nix print-dev-env --json` — what a dev shell *contains*, as data.
//!
//! Split from the module that *applies* it because the two fail differently. Everything here is a
//! pure function of one JSON string and is tested against fixtures; the other half touches the
//! environment, the filesystem and a cache. The trap documented in the parent — that the bash form
//! and the `--json` form do not contain the same things — lives on this side of the line.

/// Variables that must never be taken from a dev shell into the shell you are using.
///
/// The first five are what nix itself withholds from the bash form. The rest are the remainder of
/// nix's own `ignoreVars` (`src/nix/develop.cc`) — absent from this flake, but present in others,
/// and each one would be a different flavour of broken: `PWD` and `OLDPWD` would lie about where
/// you are, `SHELL` would point at the store's bash, `TMPDIR` at a build directory that no longer
/// exists, `SHLVL` would corrupt the nesting count.
pub(super) const IGNORED: &[&str] = &[
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

/// The shell functions a dev shell defines, from `nix print-dev-env --json`.
///
/// **A separate top-level key from `variables`**, which is easy to miss and was: `bashFunctions`
/// held 110 entries for one ordinary flake while oslo read none of them. They are stdenv's build
/// system — `genericBuild`, `runHook`, every `*Phase`, `substituteInPlace`, `patchShebangs` — and
/// without them a dev shell is a set of paths rather than a place you can build in.
///
/// The value is the body alone: no name, no braces, leading newline. Reconstructing the definition
/// is this function's whole job, and getting it wrong is silent — a body pasted without braces
/// parses as a *list of commands* that then runs on import.
pub(super) fn functions_from(json: &str) -> Result<Vec<(String, String)>, String> {
    let parsed: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let Some(functions) = parsed.get("bashFunctions").and_then(|v| v.as_object()) else {
        // An older nix, or a shell with none. Not an error: there is simply nothing to define.
        return Ok(Vec::new());
    };
    let mut out: Vec<(String, String)> = functions
        .iter()
        .filter(|(name, _)| usable_name(name))
        .filter_map(|(name, body)| Some((name.clone(), body.as_str()?.to_string())))
        .collect();
    out.sort();
    Ok(out)
}

/// The shell-local variables and arrays a dev shell defines, which its functions read.
///
/// **The other two thirds of `variables`, and useless without the functions.** `exported_from`
/// takes the 93 entries a child process would inherit; this takes the 32 `var` and 22 `array` ones
/// it would not. They are not environment — they are stdenv's own state — and the functions are
/// written against them: `configurePhase` holds the *name* of the phase to run, `prefix` and
/// `outputBin` are what `installPhase` writes to, and `preConfigureHooks` is the list `runHook`
/// walks. Importing the code without them left `prefix: parameter null or not set`.
///
/// Set unexported, because that is what they are in the builder — putting `preConfigureHooks` in
/// the environment of every child would be a different thing entirely.
/// A dev shell's own state: its scalars, and its arrays.
pub(super) type Locals = (Vec<(String, String)>, Vec<(String, Vec<String>)>);

pub(super) fn locals_from(json: &str) -> Result<Locals, String> {
    let parsed: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let Some(variables) = parsed.get("variables").and_then(|v| v.as_object()) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let (mut scalars, mut arrays) = (Vec::new(), Vec::new());
    for (name, entry) in variables {
        if !wanted(name) {
            continue;
        }
        match entry.get("type").and_then(|t| t.as_str()) {
            Some("var") => {
                if let Some(value) = entry.get("value").and_then(|v| v.as_str()) {
                    scalars.push((name.clone(), value.to_string()));
                }
            }
            Some("array") => {
                if let Some(values) = entry.get("value").and_then(|v| v.as_array()) {
                    let values: Vec<String> = values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                    arrays.push((name.clone(), values));
                }
            }
            _ => {}
        }
    }
    scalars.sort();
    arrays.sort();
    Ok((scalars, arrays))
}

/// Whether a name can be a shell function's here.
///
/// nix emits what bash accepted, and bash accepts names oslo's parser will not — `-` and `.` among
/// them, from packages that define `pkg-config_hook`-shaped helpers. One that cannot be defined is
/// skipped rather than allowed to fail the whole import.
pub(super) fn usable_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}
