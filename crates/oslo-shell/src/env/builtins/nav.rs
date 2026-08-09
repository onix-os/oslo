//! `nav` — browse directories and leave the shell in the selected one.

use crate::env::Environment;
use oslo_base::error::Result;
use oslo_ui::ask::Preset;
use oslo_ui::ask::chrome::Chrome;
use oslo_ui::nav::{Navigator, Outcome};
use oslo_ui::{scanner::Scanner, settings, theme};
use std::path::PathBuf;

pub fn builtin_nav(env: &mut Environment, args: &[String]) -> Result<i32> {
    let start = match operand(args) {
        Ok(Some(path)) => PathBuf::from(path),
        Ok(None) => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        Err(status) => return Ok(status),
    };
    if !start.is_dir() {
        eprintln!("oslo: nav: {}: not a directory", start.display());
        return Ok(1);
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
            eprintln!("oslo: nav: no terminal available");
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
                eprintln!("oslo: nav: {value}: unknown option");
                return Err(2);
            }
            value if path.is_none() => path = Some(value),
            _ => {
                eprintln!("oslo: nav: too many arguments");
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

    #[test]
    fn help_and_arguments_are_checked_without_a_terminal() {
        assert_eq!(run(&["nav", "--help"]), 0);
        assert_eq!(run(&["nav", "--unknown"]), 2);
        assert_eq!(run(&["nav", "/no/such/directory"]), 1);
        assert_eq!(run(&["nav", ".", "."]), 2);
    }
}
