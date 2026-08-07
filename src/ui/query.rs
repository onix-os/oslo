//! Compatibility access to terminal background detection.

pub use super::term::query::{Background, parse_background};

/// Return the background hint or bounded OSC 11 reply.
pub fn background() -> Option<Background> {
    if let Some(background) = super::term::query::background_from_environment() {
        return Some(background);
    }
    let tty = super::term::Tty::open()?;
    let (background, pending) = super::term::query::background_on(tty.fd(), 100);
    super::term::query::preserve_startup_input(pending);
    background
}
