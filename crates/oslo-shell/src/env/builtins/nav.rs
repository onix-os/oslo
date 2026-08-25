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

    let all = settings::current();
    let configured = &all.builtin.nav;

    // A browser named in the config wins. `nav`'s job is to leave the shell in the directory you
    // picked, not to draw the thing that picks it — so this runs it and reads the answer back:
    // same operand, same exit status, somebody else's UI.
    if !configured.command.is_empty() {
        return run_navigator(
            env,
            &configured.command,
            &start,
            configured.width,
            configured.height,
        );
    }
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

/// The viewport a browser is given when `oslo.builtin.nav` leaves the size at zero.
///
/// A navigator is a panel, not a screen: full width puts the name you are reading at one end of a
/// long empty row. These are the same two numbers the builtin browser reads, so changing them in
/// the config changes both.
const DEFAULT_WIDTH: usize = 60;
const DEFAULT_HEIGHT: usize = 50;

/// Run the configured browser, then go where it says.
///
/// The argv is the config's, verbatim but for the placeholders — see
/// [`oslo_ui::settings::nav::Nav::command`] for what they are and why they substitute *inside* an
/// argument rather than only as whole words.
fn run_navigator(
    env: &mut Environment,
    command: &[String],
    start: &Path,
    width: usize,
    height: usize,
) -> Result<i32> {
    // The browser answers by writing the directory it finished in, and the shell cd's to whatever
    // it reads back. A predictable path under /tmp would therefore let anyone who can create a file
    // there choose this shell's working directory, so the answer is written inside a private
    // 0700 directory created for this one run.
    let Ok(private) = mkdtemp(&std::env::temp_dir().join("oslo-nav-XXXXXX")) else {
        eprintln!("{}nav: could not create a private directory", origin_now());
        return Ok(2);
    };
    let answer = private.join("cwd");

    let width = if width == 0 { DEFAULT_WIDTH } else { width };
    let height = if height == 0 { DEFAULT_HEIGHT } else { height };
    let filled: Vec<String> = command
        .iter()
        .map(|word| {
            fill(
                word,
                &answer.to_string_lossy(),
                &start.to_string_lossy(),
                width,
                height,
            )
        })
        .collect();
    let (program, rest) = filled.split_first().expect("a non-empty command");

    let status = std::process::Command::new(program).args(rest).status();

    let written = std::fs::read_to_string(&answer).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&private);

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => return Ok(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("{}nav: {program}: {error}", origin_now());
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

/// The placeholders, substituted wherever they appear in one argument.
///
/// **Inside the word, not only as the whole of it.** Launching a browser inside a terminal mux
/// means a nested command line — `--command "trek --cwd-file {answer} {dir}"` — and a substitution
/// that only replaced whole arguments would hand that string through untouched, leaving the program
/// that finally runs with two braces and nowhere to write its answer.
fn fill(word: &str, answer: &str, dir: &str, width: usize, height: usize) -> String {
    word.replace("{answer}", answer)
        .replace("{dir}", dir)
        .replace("{width}", &width.to_string())
        .replace("{height}", &height.to_string())
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

#[cfg(test)]
mod command_tests {
    use super::fill;

    /// Every placeholder, and each one wherever it appears in the word.
    #[test]
    fn the_placeholders_are_substituted() {
        assert_eq!(fill("{dir}", "/a", "/start", 60, 50), "/start");
        assert_eq!(fill("{answer}", "/a", "/start", 60, 50), "/a");
        assert_eq!(fill("{width}x{height}", "/a", "/s", 60, 50), "60x50");
        assert_eq!(fill("--flag", "/a", "/s", 60, 50), "--flag", "left alone");
    }

    /// **Inside a word, not only as the whole of it.** A browser launched inside a terminal mux
    /// arrives as a nested command line, and a substitution that only replaced whole arguments
    /// would hand those braces through to the program that finally runs.
    #[test]
    fn a_nested_command_line_carries_them_through() {
        let word = fill(
            "trek --explore --cwd-file {answer} --width {width} {dir}",
            "/run/answer",
            "/home/me",
            72,
            40,
        );
        assert_eq!(
            word,
            "trek --explore --cwd-file /run/answer --width 72 /home/me"
        );
        assert!(!word.contains('{'), "nothing may be left unsubstituted");
    }

    /// A name that merely looks like one is not one, so a directory with braces in it survives.
    #[test]
    fn only_the_known_names_substitute() {
        assert_eq!(fill("{elsewhere}", "/a", "/s", 60, 50), "{elsewhere}");
        assert_eq!(
            fill("/tmp/{dir}s", "/a", "/s", 60, 50),
            "/tmp/{dir}s".replace("{dir}", "/s")
        );
    }
}
