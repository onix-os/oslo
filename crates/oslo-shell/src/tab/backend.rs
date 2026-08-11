//! Where tabs are kept, and the two answers oslo has for it.
//!
//! ```text
//!   daemon = false                        daemon = true
//!
//!   client ──unix socket──► keeper        client ──unix socket──► daemon
//!                             │ pty                                 │
//!                             ▼                                     ├─► keeper ─► oslo
//!                           oslo                                    └─► keeper ─► oslo
//!
//!   list  = read the directory            list  = ask the daemon
//!   alive = probe the lock                alive = the daemon knows
//! ```
//!
//! **The keeper is the same process in both.** What the daemon changes is who a client talks to and
//! who answers "which tabs are there" — not how a shell is held. That is why the second backend is
//! small: it is a registry and a splice, in front of machinery that already worked.
//!
//! Which one is in use is read once, where the key is pressed, so a session that started before the
//! setting changed keeps the answer it started with rather than half of each.

use super::{daemon, keeper, store};
use std::io;
use std::os::unix::net::UnixStream;

/// What the finder needs of wherever tabs are kept.
pub trait Tabs {
    /// Every tab there is, newest first.
    fn list(&self) -> io::Result<Vec<String>>;
    /// Make `name` if nothing is holding it. In the process that becomes the shell this does not
    /// return — see `keeper::Role`.
    fn ensure(&self, name: &str) -> io::Result<()>;
    /// A stream carrying the tab's bytes, ready to be pumped.
    fn connect(&self, name: &str) -> io::Result<UnixStream>;
}

/// The backend the settings ask for.
pub fn current(daemon: bool) -> Box<dyn Tabs> {
    if daemon {
        Box::new(Daemon)
    } else {
        Box::new(Direct)
    }
}

/// A keeper per tab, and the runtime directory as the registry.
pub struct Direct;

impl Tabs for Direct {
    fn list(&self) -> io::Result<Vec<String>> {
        Ok(store::list()?.into_iter().map(|(name, _)| name).collect())
    }

    fn ensure(&self, name: &str) -> io::Result<()> {
        if store::alive(name) {
            return Ok(());
        }
        super::enter::become_shell_or(
            keeper::spawn(name, oslo_ui::settings::current().tab.log_bytes)?,
            name,
        )
    }

    fn connect(&self, name: &str) -> io::Result<UnixStream> {
        super::client::connect(&store::Paths::new(name).sock(), name)
    }
}

/// One process in front of every tab, as tab-rs does it.
pub struct Daemon;

impl Tabs for Daemon {
    fn list(&self) -> io::Result<Vec<String>> {
        daemon::ask_list()
    }

    /// **Nothing to do.** The daemon makes a tab it is asked to attach to, which is what keeps the
    /// client out of the business of forking shells in this mode.
    fn ensure(&self, _name: &str) -> io::Result<()> {
        Ok(())
    }

    fn connect(&self, name: &str) -> io::Result<UnixStream> {
        daemon::attach_through(name)
    }
}
