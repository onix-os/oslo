//! Who a store encrypts to.
//!
//! A recipient is kept as the text it was written as, and parsed when something is being encrypted.
//! That is deliberate: `oslo secret where` can show you a list this binary cannot use, which is the
//! only way a person finds out that the store they are looking at was written by a shell built with
//! something theirs was not.
//!
//! # Only the native kind
//!
//! `age1…` x25519 recipients, and nothing else. age has a plugin mechanism for hardware keys where
//! an external `age-plugin-NAME` binary does the crypto, and oslo does not speak it — see
//! `docs/features/secrets.md`. A recipient this binary cannot use is named in the error rather than
//! skipped, because skipping one silently means a file encrypted to fewer keys than its owner
//! believes.

use age::x25519;

/// One party a store's files are encrypted to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient(String);

impl Recipient {
    /// Take it as written, checking only what is cheap to check.
    pub fn new(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("a recipient needs to be something".to_string());
        }
        if text.split_whitespace().count() > 1 {
            return Err(format!("{text:?} is not one recipient"));
        }
        Ok(Recipient(text.to_string()))
    }

    /// As written.
    pub fn text(&self) -> &str {
        &self.0
    }

    /// The x25519 recipient this is, or why it is not one.
    pub fn native(&self) -> Result<x25519::Recipient, String> {
        self.0.parse::<x25519::Recipient>().map_err(|e| {
            if self.0.starts_with("age1") && self.0[4..].contains('1') {
                format!(
                    "{}: an age plugin recipient. oslo does not speak the age plugin protocol; \
                     hand this store's crypto to `age` itself with `oslo secret cipher`",
                    self.0
                )
            } else {
                format!("{}: {e}", self.0)
            }
        })
    }
}

impl std::fmt::Display for Recipient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
