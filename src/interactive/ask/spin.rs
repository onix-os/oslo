//! `ui spin` — run something slow and say so while it runs.
//!
//! The only widget that takes no input. It exists because a script that shells out for ten seconds
//! looks identical to one that has hung, and the fix people reach for otherwise is printing a dot
//! per second into the output they are trying to capture.
//!
//! # Where everything goes
//!
//! The spinner is on **stderr** and is erased when the command finishes, so `$(ui spin -- curl …)`
//! captures the command's stdout and nothing else. The command's own exit status is the widget's.
//! Those two together are what let `ui spin` be wrapped around a command already in a script
//! without changing what the script does.
//!
//! With no terminal the command still runs — just with no spinner. A CI log does not want a
//! thousand frames of animation, and refusing to run at all would be worse.

use super::show;
use crate::interactive::theme;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The frames, and how fast they turn.
///
/// Braille rather than `|/-\`: it is one cell in every terminal that can draw it, it does not
/// jitter between frames of different widths, and it is what every spinner written this decade
/// uses. The fallback is for a terminal that cannot.
const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const PLAIN_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
const FRAME_MS: u64 = 80;

/// What to run, and what to say while it runs.
#[derive(Debug, Clone)]
pub struct Spin {
    pub title: String,
    /// The command and its arguments. Run directly rather than through a shell, so a title
    /// containing a quote cannot become part of the command.
    pub command: Vec<String>,
    /// Keep the command's output on screen rather than letting it through to the caller's stdout.
    /// Off by default, because a spinner that ate the output would make `$(ui spin …)` useless.
    pub quiet: bool,
}

/// Run it. The answer is the command's exit status.
pub fn spin(spec: &Spin) -> i32 {
    if spec.command.is_empty() {
        eprintln!("oslo: ui spin: nothing to run");
        return 2;
    }

    let drawing = std::io::IsTerminal::is_terminal(&std::io::stderr());
    let running = Arc::new(AtomicBool::new(true));
    let ticker = drawing.then(|| {
        let running = Arc::clone(&running);
        let title = spec.title.clone();
        // A thread rather than a timer on the read path: the command is a blocking wait, and the
        // spinner has to turn while it does nothing.
        std::thread::spawn(move || {
            let ui = theme::current().ui;
            let depth = theme::depth();
            let frames: &[&str] = if theme::depth() == theme::Depth::None {
                &PLAIN_FRAMES
            } else {
                &FRAMES
            };
            let mut at = 0usize;
            while running.load(Ordering::Relaxed) {
                show(&format!(
                    "\r\x1b[K{} {}",
                    ui.accent.paint(frames[at % frames.len()], depth),
                    ui.muted.paint(&title, depth)
                ));
                at += 1;
                std::thread::sleep(std::time::Duration::from_millis(FRAME_MS));
            }
            // Erase it: the spinner was a status, not output, and leaving the last frame on the
            // row would put it above whatever the script prints next.
            show("\r\x1b[K");
        })
    });

    let status = Command::new(&spec.command[0])
        .args(&spec.command[1..])
        .stdout(if spec.quiet {
            Stdio::null()
        } else {
            Stdio::inherit()
        })
        .status();

    running.store(false, Ordering::Relaxed);
    if let Some(ticker) = ticker {
        // Joined rather than detached, so the erase has certainly happened before the caller
        // prints anything — otherwise the spinner's last frame lands on top of it.
        let _ = ticker.join();
    }

    match status {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("oslo: ui spin: {}: {e}", spec.command[0]);
            127
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(command: &[&str]) -> Spin {
        Spin {
            title: "working".to_string(),
            command: command.iter().map(|s| s.to_string()).collect(),
            quiet: true,
        }
    }

    /// The command's status is the widget's, which is what lets `ui spin` wrap a command already
    /// in a script without changing what the script does.
    #[test]
    fn the_commands_status_is_the_answer() {
        assert_eq!(spin(&spec(&["true"])), 0);
        assert_eq!(spin(&spec(&["false"])), 1);
        assert_eq!(spin(&spec(&["sh", "-c", "exit 7"])), 7);
    }

    /// A command that is not there is 127, as a shell reports it.
    #[test]
    fn a_missing_command_is_reported_like_a_shell_would() {
        assert_eq!(spin(&spec(&["no-such-command-anywhere"])), 127);
    }

    #[test]
    fn nothing_to_run_is_a_usage_error() {
        assert_eq!(spin(&spec(&[])), 2);
    }

    /// It runs headless — a CI log wants no animation, but refusing to run would be worse.
    #[test]
    fn it_still_runs_without_a_terminal() {
        // Under `cargo test` stderr is not a terminal, so every case above already took this
        // path; this says so on purpose.
        assert_eq!(spin(&spec(&["true"])), 0);
    }
}
