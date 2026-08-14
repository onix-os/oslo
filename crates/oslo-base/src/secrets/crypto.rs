//! Which mechanism seals and opens a store's files.
//!
//! # Three, and the difference between them is *where they can run*
//!
//! | | does the crypto | reachable from |
//! |---|---|---|
//! | [`Crypto::Native`] | oslo's own age | anywhere — `cron`, `dash`, a `Makefile`, a container |
//! | [`Crypto::Command`] | another program, down a pipe | anywhere: it is a subprocess, not a shell |
//! | [`Crypto::Hook`] | a Lua handler on `on-secret-encrypt` | **only where Lua runs** |
//!
//! That last row is the whole reason this is an enum in a *file* rather than something Lua sets.
//! `oslo secret get` is a tool: it never builds an `Environment` and never starts an interpreter, so
//! a hook is not reachable from it. A store whose crypto were silently a hook would work at your
//! prompt and quietly do something else — or nothing — in the cron job that reads the same store.
//!
//! So the file declares *which mechanism*, and Lua supplies *the mechanism itself*. A process with
//! no Lua that meets `crypto hook` fails, loudly, saying so. Nothing silently diverges.
//!
//! # Why a hook can serve several stores
//!
//! A handler that returns `nil` means "not mine", and the next one is asked. So three plugins can
//! each answer for their own store off one hook, and a store nobody claims falls through to a
//! refusal naming it rather than to a default that would encrypt to the wrong thing.

use super::cipher::Cipher;

/// What seals and opens one store's files.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Crypto {
    /// oslo's own age, with this store's keys and recipients.
    #[default]
    Native,
    /// Another program, plaintext and ciphertext down pipes.
    Command(Cipher),
    /// A Lua handler, which exists only in a process that runs Lua.
    Hook,
}

impl Crypto {
    /// What the section said, if it said anything.
    ///
    /// `crypto hook` and an `encrypt`/`decrypt command` in one section is a contradiction rather
    /// than a precedence question: the two mechanisms have different reach, and quietly picking one
    /// would mean a store readable in one process and not the other.
    pub fn of(hooked: bool, cipher: &Cipher) -> Result<Crypto, String> {
        match (hooked, cipher.is_external()) {
            (true, true) => Err(
                "`crypto hook` and an `encrypt`/`decrypt command` in one store: pick one"
                    .to_string(),
            ),
            (true, false) => Ok(Crypto::Hook),
            (false, true) => Ok(Crypto::Command(cipher.clone())),
            (false, false) => Ok(Crypto::Native),
        }
    }

    /// Whether reaching it means running or calling something outside oslo's own age.
    pub fn is_external(&self) -> bool {
        !matches!(self, Crypto::Native)
    }

    /// How it is written in the file, when it is written at all.
    pub fn lines(&self) -> Vec<String> {
        match self {
            Crypto::Native => Vec::new(),
            Crypto::Command(cipher) => cipher.lines(),
            Crypto::Hook => vec!["crypto hook".to_string()],
        }
    }
}
