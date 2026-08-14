//! The built-in mechanism: one key, one AEAD, and nothing to configure.
//!
//! # What it is
//!
//! XChaCha20-Poly1305 with a 32-byte key kept in a file. That is the whole of it. The same AEAD
//! `age` uses for its payload, without any of the key exchange around it — because the key exchange
//! is what buys *public-key recipients*, and a shell that wants those should use `age` itself
//! through an `encrypt command`.
//!
//! ```text
//! OSLO1 │ 24-byte nonce │ ciphertext ‖ 16-byte tag
//!   5   │      24       │ …
//! ```
//!
//! **X**ChaCha rather than plain ChaCha for one reason: a 192-bit nonce can be drawn at random for
//! every write without anybody counting. With a 96-bit nonce, a random one repeats often enough to
//! matter and the counter that avoids it is state that has to survive a crash, a restore and a
//! second machine writing to the same store.
//!
//! # What it deliberately does not do
//!
//! No recipients, no public half, no passphrase, no key derivation. A store is opened by the key
//! or it is not opened. Sharing one with a colleague or a second machine means either copying the
//! key over a channel you trust, or — better — handing that store's crypto to `age`, which has
//! recipients precisely because that is a hard problem worth somebody else's code.

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

/// What every file this writes begins with.
const MAGIC: &[u8] = b"OSLO1";

/// The nonce XChaCha takes, in bytes.
const NONCE: usize = 24;

/// What a key file holds, so one is recognisable on sight and not mistaken for a secret.
const PREFIX: &str = "OSLO-KEY-1:";

/// Encrypt with `key`.
pub fn seal(key: &[u8; 32], value: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let nonce = XNonce::from(random::<NONCE>()?);
    let sealed = cipher
        .encrypt(&nonce, value)
        .map_err(|_| "the value could not be encrypted".to_string())?;

    let mut file = Vec::with_capacity(MAGIC.len() + NONCE + sealed.len());
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(&nonce);
    file.extend_from_slice(&sealed);
    Ok(file)
}

/// Decrypt with `key`, or say which way it did not work.
///
/// **A wrong key and a damaged file are told apart**, because they are different problems: one is
/// "you are holding the wrong thing" and the other is "this file is not what it was".
pub fn unseal(key: &[u8; 32], file: &[u8]) -> Result<Vec<u8>, String> {
    let Some(rest) = file.strip_prefix(MAGIC) else {
        return Err("not a file this oslo wrote: no OSLO1 header".to_string());
    };
    if rest.len() < NONCE {
        return Err("not a file this oslo wrote: it stops inside the nonce".to_string());
    }
    let (nonce, sealed) = rest.split_at(NONCE);

    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    cipher
        .decrypt(
            &XNonce::try_from(nonce).map_err(|_| "a nonce of the wrong size")?,
            sealed,
        )
        .map_err(|_| "the key did not open it, or the file has been changed".to_string())
}

/// A key, as it is written in a file.
pub fn write_key(key: &[u8; 32]) -> String {
    format!(
        "{PREFIX}{}\n",
        base64::engine::general_purpose::STANDARD.encode(key)
    )
}

/// The first key in what a file or a program gave us.
///
/// Comment lines are skipped, the way a key file people edit needs.
pub fn read_key(text: &str) -> Result<[u8; 32], String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or("no key in it")?;
    let body = line
        .strip_prefix(PREFIX)
        .ok_or_else(|| format!("a key begins with {PREFIX}, and this does not"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|_| "the key is not base64".to_string())?;
    bytes
        .try_into()
        .map_err(|_| "a key is 32 bytes, and this is not".to_string())
}

/// A new key, from the operating system's randomness and nowhere else.
pub fn generate_key() -> Result<[u8; 32], String> {
    random::<32>()
}

/// `N` bytes from the operating system.
///
/// **`getrandom`, not a seeded generator.** A nonce or a key drawn from anything this process could
/// have predicted is the one mistake in this file that would not show up in a test.
fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness from the system: {e}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_goes_in_comes_back_out() {
        let key = generate_key().expect("a key");
        for value in [b"".as_slice(), b"short", &[0u8; 100_000]] {
            let sealed = seal(&key, value).expect("seal");
            assert_ne!(&sealed[5..], value, "the value is in the file in the clear");
            assert_eq!(unseal(&key, &sealed).expect("unseal"), value);
        }
    }

    /// **Two writes of one value differ**, which is what the random nonce is for: equal files would
    /// tell anybody holding the store that a secret had not changed between two backups.
    #[test]
    fn the_same_value_seals_differently_every_time() {
        let key = generate_key().expect("a key");
        let once = seal(&key, b"the same value").expect("seal");
        let twice = seal(&key, b"the same value").expect("seal");
        assert_ne!(once, twice);
        assert_eq!(
            unseal(&key, &once).expect("a"),
            unseal(&key, &twice).expect("b")
        );
    }

    /// A wrong key does not open it, and neither does a file somebody has edited.
    #[test]
    fn it_refuses_the_wrong_key_and_a_changed_file() {
        let key = generate_key().expect("a key");
        let sealed = seal(&key, b"value").expect("seal");
        assert!(
            unseal(&generate_key().expect("another"), &sealed).is_err(),
            "wrong key opened it"
        );

        let mut bent = sealed.clone();
        let last = bent.len() - 1;
        bent[last] ^= 1;
        assert!(unseal(&key, &bent).is_err(), "a changed file opened");

        assert!(unseal(&key, b"nonsense").unwrap_err().contains("OSLO1"));
    }

    #[test]
    fn a_key_survives_being_written_down() {
        let key = generate_key().expect("a key");
        let text = write_key(&key);
        assert!(text.starts_with(PREFIX));
        assert_eq!(read_key(&text).expect("read"), key);
        assert_eq!(
            read_key(&format!("# a comment\n{text}")).expect("read"),
            key
        );
        for bad in [
            "",
            "# only a comment",
            "not-a-key",
            "OSLO-KEY-1:!!!",
            "OSLO-KEY-1:c2hvcnQ=",
        ] {
            assert!(read_key(bad).is_err(), "{bad:?} was accepted");
        }
    }
}
