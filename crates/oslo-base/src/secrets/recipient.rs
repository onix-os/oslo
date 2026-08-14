//! Who a store's files are written for.
//!
//! A recipient is kept as the text it was written as and parsed when something is encrypted. That
//! is deliberate: `oslo secret where` can show a list this binary cannot use, which is the only way
//! somebody finds out that the store in front of them was written by a shell built differently.
//!
//! The published half of a key, and nothing else. A store that wants age's recipients — hardware
//! keys, a colleague who already has an age identity — hands its crypto to `age` itself with
//! `encrypt command`, and none of this is involved.

/// One party a store's files are written for.
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

    /// The public key this is, or why it is not one.
    #[cfg(feature = "crypt")]
    pub fn public(&self) -> Result<[u8; 32], String> {
        super::native::read_public(&self.0).map_err(|e| format!("{}: {e}", self.0))
    }
}

impl std::fmt::Display for Recipient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
