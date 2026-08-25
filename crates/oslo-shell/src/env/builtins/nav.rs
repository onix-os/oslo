//! `nav` — browse directories and leave the shell in the selected one.

use crate::env::Environment;
use crate::env::origin_now;
use nix::unistd::mkdtemp;
use oslo_base::error::Result;
use oslo_ui::ask::Preset;
use oslo_ui::ask::chrome::Chrome;
use oslo_ui::nav::{Navigator, Outcome};
use oslo_ui::{scanner::Scanner, settings, theme};
use std::path::{Path, PathBuf};

pub fn builtin_nav(env: &mut Environment, args: &[String]) -> Result<i32> {
    let start = match operand(args) {
        Ok(Some(path)) => PathBuf::from(path),
        Ok(None) => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        Err(status) => return Ok(status),
    };
    if !start.is_dir() {
        eprintln!("{}nav: {}: not a directory", origin_now(), start.display());
        return Ok(1);
    }

    // A better navigator on $PATH wins. `nav`'s job is to leave the shell in the directory
    // you picked, not to draw the browser that picks it, so when one is installed this builtin
    // runs it and reads the answer back -- same operand, same exit status, somebody else's UI.
    if let Some(result) = delegated(env, &start) {
        return result;
    }

    let all = settings::current();
    let configured = &all.builtin.nav;
    let mut look = Preset::History.look();
    look.filter_at = configured.filter_at;
    look.reverse = configured.reverse;
    look.placeholder = "type to filter".to_string();
    if !configured.scanner {
        look.scanner = None;
    } else if look.scanner.is_none() {
        look.scanner = Some(Scanner::default());
    }
    let chrome = Chrome {
        legend: configured.legend,
        border: configured.border,
        border_style: configured
            .border_fg
            .map(theme::Style::fg)
            .unwrap_or_default(),
        fit: configured.border_fit,
        legend_gap: configured.legend_gap,
        padding_x: configured.padding_x,
        padding_y: configured.padding_y,
        fullscreen: configured.fullscreen,
        align_x: oslo_ui::ask::chrome::Place::Center,
        align_y: configured.position,
    };
    let spec = Navigator {
        start,
        hidden: configured.hidden,
        width: configured.width,
        height: configured.height,
        fuzzy: all.completion.fuzzy,
        icons: configured.icons.clone(),
        type_nav: configured.type_nav,
        chrome,
        look,
    };

    let outcome = oslo_ui::nav::open(&spec, |path| {
        let args = vec![
            "rm".to_string(),
            "--".to_string(),
            path.to_string_lossy().into_owned(),
        ];
        super::remove::builtin_rm(env, &args).is_ok_and(|status| status == 0)
    });
    match outcome {
        Outcome::ChangeTo(path) => change_directory(env, path),
        Outcome::Cancelled => Ok(1),
        Outcome::NoTerminal => {
            eprintln!("{}nav: no terminal available", origin_now());
            Ok(2)
        }
    }
}

fn operand(args: &[String]) -> std::result::Result<Option<&str>, i32> {
    let mut path = None;
    let mut options = true;
    for arg in &args[1..] {
        match arg.as_str() {
            "-h" | "--help" if options => {
                println!("usage: nav [path]");
                println!("type to filter; arrows browse; Enter opens; Delete removes; Esc exits");
                return Err(0);
            }
            "--" if options => options = false,
            value if options && value.starts_with('-') => {
                eprintln!("{}nav: {value}: unknown option", origin_now());
                return Err(2);
            }
            value if path.is_none() => path = Some(value),
            _ => {
                eprintln!("{}nav: too many arguments", origin_now());
                return Err(2);
            }
        }
    }
    Ok(path)
}

fn change_directory(env: &mut Environment, path: PathBuf) -> Result<i32> {
    if std::env::current_dir().is_ok_and(|current| current == path) {
        return Ok(0);
    }
    crate::env::builtins::builtin_cd(
        env,
        &["cd".to_string(), path.to_string_lossy().into_owned()],
    )
}

/// The navigator trek provides, when it is installed.
///
/// `None` means there is nothing to delegate to and the built-in browser should run. Resolution
/// goes through the shell's own `PATH` table, so hiding `trek` in a directory hides it from `nav`
/// too — that is the way back to the built-in browser without uninstalling anything.
fn delegated(env: &mut Environment, start: &Path) -> Option<Result<i32>> {
    let program = super::hash::lookup("trek")?;
    Some(run_navigator(env, &program, start))
}

/// Run it, then go where it says.
fn run_navigator(env: &mut Environment, program: &Path, start: &Path) -> Result<i32> {
    // trek answers by writing the directory it finished in, and the shell cd's to whatever it
    // reads back. A predictable path under /tmp would therefore let anyone who can create a file
    // there choose this shell's working directory, so the answer is written inside a private
    // 0700 directory created for this one run.
    let Ok(private) = mkdtemp(&std::env::temp_dir().join("oslo-nav-XXXXXX")) else {
        eprintln!("{}nav: could not create a private directory", origin_now());
        return Ok(2);
    };
    let answer = private.join("cwd");

    let status = std::process::Command::new(program)
        .arg("--explore")
        .arg("--cwd-file")
        .arg(&answer)
        .arg(start)
        .status();

    let written = std::fs::read_to_string(&answer).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&private);

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => return Ok(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("{}nav: {}: {error}", origin_now(), program.display());
            return Ok(127);
        }
    }

    match chosen(&written) {
        Some(path) => change_directory(env, path),
        // Nowhere to go is a cancelled navigation, which is what the built-in browser
        // reports when you leave it with Esc.
        None => Ok(1),
    }
}

/// The directory trek wrote, if it is one.
///
/// The trailing newline matters: `cd` to a path with one appended fails with a message naming a
/// directory that visibly exists, which is a genuinely confusing way to be told about a stray byte.
fn chosen(written: &str) -> Option<PathBuf> {
    let trimmed = written.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    path.is_dir().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(words: &[&str]) -> i32 {
        let mut env = Environment::new();
        let args = words
            .iter()
            .map(|word| word.to_string())
            .collect::<Vec<_>>();
        builtin_nav(&mut env, &args).expect("nav status")
    }

    // The realistic failure is the trailing newline: `cd` would refuse a path that visibly
    // exists, naming it with an invisible byte on the end.
    #[test]
    fn an_answer_is_a_directory_or_it_is_nothing() {
        assert_eq!(chosen(""), None);
        assert_eq!(chosen("   \n"), None);
        assert_eq!(chosen("/no/such/directory"), None);
        assert_eq!(chosen("/tmp\n"), Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn help_and_arguments_are_checked_without_a_terminal() {
        assert_eq!(run(&["nav", "--help"]), 0);
        assert_eq!(run(&["nav", "--unknown"]), 2);
        assert_eq!(run(&["nav", "/no/such/directory"]), 1);
        assert_eq!(run(&["nav", ".", "."]), 2);
    }
}
