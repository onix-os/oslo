//! What is wrong with this installation, said once at the top of `--help`.
//!
//! # Why these checks and not others
//!
//! Every one of them is something you **cannot see from inside a working shell**. oslo runs
//! perfectly well with `/bin/sh` still pointing at dash — you just never get it for anything that
//! starts a script, which is most of what a system shell does. It runs perfectly well when nobody
//! but you can execute the binary — until a service, a cron job or another user tries.
//!
//! A check whose failure is already obvious does not belong here. The list will grow; the rule it
//! grows by is that one.
//!
//! # Reported, never fixed
//!
//! Nothing here writes anything. Repointing `/bin/sh` is a decision about the whole machine, taken
//! with the package manager and a way back — not something a `--help` should do because it noticed.
//! Each line therefore says what is true, and the box says how to act on it.
//!
//! # A hint is not a warning
//!
//! Two kinds of finding, and they are drawn as two boxes because the difference matters.
//!
//! * **A hint** is a fact about this machine you may not have chosen: `/bin/sh` still points at
//!   dash. Nothing is broken — oslo runs, your scripts run, they just do not run *in oslo*. It is
//!   information, and drawing it in red taught the eye to ignore the colour for the day something
//!   actually was wrong.
//! * **A warning** is something that fails for somebody: a binary no other user can execute means
//!   their `#!/bin/sh` dies with a permission error naming the script rather than the shell.

use super::help::Paint;
use std::path::{Path, PathBuf};

/// The paths a system expects to find a shell at.
///
/// `/bin/sh` is the one POSIX names and every shebang uses. `/usr/bin/sh` matters on the
/// merged-`/usr` distributions where `/bin` is itself a symlink — there the two are usually the
/// same file, and a system where they are not is a system where half the scripts get a different
/// shell from the other half.
const SH_PATHS: &[&str] = &["/bin/sh", "/usr/bin/sh"];

/// What the checks found, as plain data.
///
/// Separated from the wording **so the wording can be tested without a filesystem**. A test that
/// had to create `/bin/sh` to check the sentence about it would be a test that cannot run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Facts {
    /// Each `sh` path that exists and does *not* resolve to this binary, with what it does resolve
    /// to. A path that is absent is not listed: a system with no `/usr/bin/sh` is not misconfigured.
    pub foreign_sh: Vec<(String, String)>,
    /// The running binary's permission bits, when they are not what a system shell needs.
    pub bad_mode: Option<u32>,
}

/// How much a finding is worth: something that fails, or something you may not have meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Nothing is broken. See the module note.
    Hint,
    /// Something fails for somebody.
    Warning,
}

impl Level {
    /// The word in the box's top edge, padded as it is drawn.
    fn title(self) -> &'static str {
        match self {
            Level::Hint => " hint ",
            Level::Warning => " warning ",
        }
    }
}

impl Facts {
    /// One line per finding, with how much it matters, worst first.
    pub fn findings(&self) -> Vec<(Level, String)> {
        let mut found = Vec::new();
        if let Some(mode) = self.bad_mode {
            found.push((
                Level::Warning,
                format!("this binary is mode {mode:04o}; a system shell needs 0755"),
            ));
        }
        for (path, points_at) in &self.foreign_sh {
            // A hint: oslo runs perfectly well beside another `/bin/sh`. What you do not get is
            // oslo for the things that start a script, which is a choice rather than a fault.
            found.push((Level::Hint, format!("{path} is {points_at}, not oslo")));
        }
        found
    }
}

/// Whether the config wants these warnings at all.
///
/// **The config has to be run to find out**, because it is Lua and the value could be computed.
/// `--help` is parsed long before the shell would otherwise load it, so this loads it here.
///
/// That is a real cost — it is the user's own code, and a config that prints puts its output above
/// the help — and it is the only honest way for `oslo.misc.warnings = false` to mean anything on
/// this path. It is paid only when a config file exists, and `--help` is not a hot path.
///
/// Anything that goes wrong leaves the warnings **on**. A failure to read the setting must not
/// silence the thing the setting was about.
pub fn wanted() -> bool {
    use oslo::env::Environment;
    use std::sync::{Arc, Mutex};

    let env = Arc::new(Mutex::new(Environment::new()));
    let files = crate::startup::lua_init::config_files(&env.lock().expect("a fresh environment"));
    if files.is_empty() {
        return true;
    }
    let Ok(lua) = oslo::LuaEngine::new() else {
        return true;
    };
    if !crate::startup::lua_init::install_bindings(&lua, Arc::clone(&env)) {
        return true;
    }
    for path in &files {
        crate::startup::lua_init::load_config(&lua, path);
    }
    // Complaints are dropped rather than printed: the shell itself reports them properly a moment
    // later, and saying them twice from `--help` would be noise attached to the wrong command.
    lua.read_settings().0.misc.warnings
}

/// Look at the real filesystem.
///
/// **One line per problem, not per path.** `/bin` is a symlink to `usr/bin` on every distribution
/// that did the usrmerge, so `/bin/sh` and `/usr/bin/sh` are the same inode — reporting both said
/// the same thing twice and made one misconfiguration look like two. They are reported separately
/// only where they genuinely differ, which is a system where half the scripts would get a
/// different shell from the other half and is worth two lines.
pub fn detect() -> Facts {
    let me = std::fs::canonicalize("/proc/self/exe").ok();
    let mut foreign_sh: Vec<(String, String)> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for path in SH_PATHS {
        let path = Path::new(path);
        // Keyed on where it *resolves*, so the second name for one file is skipped.
        let Ok(target) = std::fs::canonicalize(path) else {
            continue;
        };
        if seen.contains(&target) {
            continue;
        }
        seen.push(target);
        foreign_sh.extend(foreign(path, me.as_deref()));
    }
    Facts {
        foreign_sh,
        bad_mode: me.as_deref().and_then(bad_mode),
    }
}

/// `Some((path, what it really is))` when `path` exists and is not us.
fn foreign(path: &Path, me: Option<&Path>) -> Option<(String, String)> {
    // A path that is not there at all is not a misconfiguration — plenty of systems have no
    // `/usr/bin/sh` — so only a resolvable one is judged.
    let target = std::fs::canonicalize(path).ok()?;
    if Some(target.as_path()) == me {
        return None;
    }
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("something else");
    Some((path.display().to_string(), name.to_string()))
}

/// The mode, when it is not one a system shell can be run under.
///
/// The test is **executable by everyone**, not an exact `0755`: a distribution that ships it
/// `0555` is right, and complaining would be noise. What actually breaks is a missing `x` for
/// group or other, because then every other user's `#!/bin/sh` fails with a permission error that
/// names the script rather than the shell.
fn bad_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o7777;
    (mode & 0o111 != 0o111).then_some(mode)
}

/// The colour a warning is drawn in.
///
/// True red, given as RGB and left to [`crate::ui::theme::Depth`] to fold down for a
/// terminal that cannot take it — 256 quantises it, 16 lands on plain red, and a `NO_COLOR` run
/// gets the box with no colour at all. A warning has to survive all four.
const RED: (u8, u8, u8) = (224, 64, 64);

/// The box, or nothing at all when nothing is wrong.
///
/// **Silent when there is nothing to say.** A warning box that appears every time teaches people
/// to skip the top of the output, which costs exactly the one time it matters.
///
/// `width` is how wide the page it sits under is. The box is drawn to match, because a box
/// narrower than the text above it reads as a stray fragment rather than as the page's own
/// footer — and it still grows past `width` rather than truncate, since a warning that has been
/// cut off is worse than a ragged edge.
pub fn box_of(facts: &Facts, paint: Paint, width: usize) -> String {
    let findings = facts.findings();
    let mut s = String::new();
    // Worst first, and each level in its own box: one box titled for the more serious of two
    // different things would be lying about the other one.
    for level in [Level::Warning, Level::Hint] {
        let lines: Vec<&str> = findings
            .iter()
            .filter(|(found, _)| *found == level)
            .map(|(_, line)| line.as_str())
            .collect();
        if !lines.is_empty() {
            s.push_str(&drawn(level, &lines, paint, width));
        }
    }
    s
}

/// One box: a titled top edge, a row per line, a bottom edge, and a blank row after it.
fn drawn(level: Level, lines: &[&str], paint: Paint, width: usize) -> String {
    // A hint is dim rather than coloured. Red is what makes the eye stop, and spending it on
    // something that is not wrong is how a warning stops being read at all.
    let ink: Box<dyn Fn(&str) -> String> = match level {
        Level::Warning => Box::new(paint.rgb(RED)),
        Level::Hint => Box::new(move |text: &str| paint.dim(text)),
    };
    let title = level.title();

    // The interior, between the two corner characters — so a drawn row is `inner + 2` wide, and
    // matching a page of `width` means an interior of `width - 2`.
    //
    // The content's own `+ 2` is its one space of padding on each side, and is applied *before*
    // the page is taken into account: doing it after made every box two columns too wide.
    let inner = (lines
        .iter()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0)
        .max(title.len())
        + 2)
    .max(width.saturating_sub(2));

    let mut s = String::new();
    let rule = "─".repeat(inner - title.len() - 1);
    s.push_str(&ink(&format!("╭─{title}{rule}╮\n")));
    for line in lines {
        let pad = " ".repeat(inner - 1 - line.chars().count());
        s.push_str(&format!("{} {line}{pad}{}\n", ink("│"), ink("│")));
    }
    s.push_str(&ink(&format!("╰{}╯\n", "─".repeat(inner))));
    s.push('\n');
    s
}

#[cfg(test)]
#[path = "warn/tests.rs"]
mod tests;
