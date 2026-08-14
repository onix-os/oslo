//! The built-in mechanism: a key you keep, recipients you publish.
//!
//! # The shape, and why it is this shape
//!
//! A **sealed box**, which is the construction age uses and NaCl calls `crypto_box_seal`. One
//! random *file key* encrypts the value; that file key is then wrapped once per recipient, each
//! wrap using a fresh ephemeral X25519 keypair against that recipient's public half.
//!
//! ```text
//! OSLO2 │ n │ n × [ ephemeral public (32) │ wrapped file key (48) ] │ nonce (24) │ ciphertext ‖ tag
//! ```
//!
//! That indirection buys the thing a single symmetric key cannot have: **a store can be readable by
//! several keys without any of them being shared.** Your laptop keeps one secret, the server keeps
//! another, and the store lists two recipients — nobody hands anybody a private key, and taking a
//! machine off the list is one `recipient rm` and a `rotate`.
//!
//! # What is deliberately not here
//!
//! Not the age *format*. `age -d` will not open these files and is not meant to: what was worth
//! having was the key model, and carrying the format meant bech32, scrypt, a localised error
//! catalogue and thirty-two packages. A store that wants real age files says so —
//! `encrypt command age -R …` — and this module is not involved at all.
//!
//! No passphrase, and no key derivation from one: the secret is a file at mode `0600`, like
//! `~/.ssh/id_ed25519`.

use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use x25519_dalek::{PublicKey, StaticSecret};

/// What every file this writes begins with.
const MAGIC: &[u8] = b"OSLO2";

/// The nonce XChaCha takes.
const NONCE: usize = 24;

/// An X25519 public or secret key.
const KEY: usize = 32;

/// A wrapped file key: 32 bytes of key and the AEAD's 16-byte tag.
const WRAPPED: usize = KEY + 16;

/// One recipient's share: the ephemeral public half, and the file key wrapped to it.
const STANZA: usize = KEY + WRAPPED;

/// What a secret key file holds, so one is recognisable on sight.
const SECRET: &str = "OSLO-KEY-1:";

/// What a recipient is written as. A different prefix from the secret, because the whole point is
/// that one of them is safe to publish and the other is not.
const PUBLIC: &str = "OSLO-PUB-1:";

/// Domain separation for the wrapping key, so this construction's output can never be confused with
/// another use of the same X25519 secret.
const INFO: &[u8] = b"oslo secret v2 wrap";

/// Encrypt to every recipient.
///
/// **Every recipient, or none.** One that cannot be used fails the whole write rather than being
/// skipped: a file quietly encrypted to fewer keys than its owner listed is the failure here that
/// stays invisible until the day one of the others is the only key left.
pub fn seal(recipients: &[[u8; KEY]], value: &[u8]) -> Result<Vec<u8>, String> {
    if recipients.is_empty() {
        return Err("no recipient to encrypt to".to_string());
    }
    if recipients.len() > u8::MAX as usize {
        return Err(format!("{} recipients is more than 255", recipients.len()));
    }

    let file_key: [u8; KEY] = random()?;
    let mut out = Vec::with_capacity(MAGIC.len() + 1 + recipients.len() * STANZA + NONCE);
    out.extend_from_slice(MAGIC);
    out.push(recipients.len() as u8);

    for recipient in recipients {
        let ephemeral = StaticSecret::from(random::<KEY>()?);
        let public = PublicKey::from(&ephemeral);
        let shared = ephemeral.diffie_hellman(&PublicKey::from(*recipient));
        let wrapping = derive(shared.as_bytes(), public.as_bytes(), recipient)?;

        // A zero nonce is safe *here* and nowhere else: the wrapping key comes from a fresh
        // ephemeral secret, so it encrypts exactly one message and can never repeat.
        let wrapped = XChaCha20Poly1305::new(&Key::from(wrapping))
            .encrypt(&XNonce::from([0u8; NONCE]), file_key.as_slice())
            .map_err(|_| "the file key could not be wrapped".to_string())?;
        out.extend_from_slice(public.as_bytes());
        out.extend_from_slice(&wrapped);
    }

    let nonce = XNonce::from(random::<NONCE>()?);
    let sealed = XChaCha20Poly1305::new(&Key::from(file_key))
        .encrypt(&nonce, value)
        .map_err(|_| "the value could not be encrypted".to_string())?;
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Ok(out)
}

/// Decrypt with `secret`, if its public half is one of the recipients.
pub fn unseal(secret: &[u8; KEY], file: &[u8]) -> Result<Vec<u8>, String> {
    let rest = file
        .strip_prefix(MAGIC)
        .ok_or("not a file this oslo wrote: no OSLO2 header")?;
    let (count, rest) = rest.split_first().ok_or("it stops before the recipients")?;

    let wrapped = STANZA * *count as usize;
    if rest.len() < wrapped + NONCE {
        return Err("it stops inside the recipients".to_string());
    }
    let (stanzas, body) = rest.split_at(wrapped);
    let (nonce, sealed) = body.split_at(NONCE);

    let secret = StaticSecret::from(*secret);
    let mine = PublicKey::from(&secret);
    for stanza in stanzas.chunks_exact(STANZA) {
        let (ephemeral, wrapped) = stanza.split_at(KEY);
        let ephemeral: [u8; KEY] = ephemeral.try_into().expect("a chunk of exactly 32");
        let shared = secret.diffie_hellman(&PublicKey::from(ephemeral));
        let wrapping = derive(shared.as_bytes(), &ephemeral, mine.as_bytes())?;

        // Somebody else's stanza: the tag fails, which is the answer rather than an error.
        let Ok(file_key) = XChaCha20Poly1305::new(&Key::from(wrapping))
            .decrypt(&XNonce::from([0u8; NONCE]), wrapped)
        else {
            continue;
        };
        let file_key: [u8; KEY] = file_key
            .try_into()
            .map_err(|_| "a wrapped file key of the wrong size".to_string())?;
        return XChaCha20Poly1305::new(&Key::from(file_key))
            .decrypt(
                &XNonce::try_from(nonce).map_err(|_| "a nonce of the wrong size")?,
                sealed,
            )
            .map_err(|_| "the file has been changed since it was written".to_string());
    }
    Err("no key of yours is a recipient of this file".to_string())
}

/// The wrapping key for one ephemeral-to-recipient pair.
///
/// **Hashed, never used raw.** An X25519 shared secret is a curve point rather than a uniform key,
/// and the salt binds it to the two public halves it came from, so a wrap cannot be replayed
/// against a different pair.
fn derive(
    shared: &[u8; KEY],
    ephemeral: &[u8; KEY],
    recipient: &[u8; KEY],
) -> Result<[u8; KEY], String> {
    let mut salt = [0u8; KEY * 2];
    salt[..KEY].copy_from_slice(ephemeral);
    salt[KEY..].copy_from_slice(recipient);

    let mut key = [0u8; KEY];
    hkdf::Hkdf::<sha2::Sha256>::new(Some(&salt), shared)
        .expand(INFO, &mut key)
        .map_err(|_| "the wrapping key could not be derived".to_string())?;
    Ok(key)
}

/// A secret key, as it is written in a file.
pub fn write_secret(secret: &[u8; KEY]) -> String {
    format!("{SECRET}{}\n", encode(secret))
}

/// A recipient, as it is written in `secrets.conf` or handed to somebody.
pub fn write_public(public: &[u8; KEY]) -> String {
    format!("{PUBLIC}{}", encode(public))
}

/// The public half of a secret key.
pub fn public_of(secret: &[u8; KEY]) -> [u8; KEY] {
    PublicKey::from(&StaticSecret::from(*secret)).to_bytes()
}

/// The first secret key in what a file or a program gave us.
pub fn read_secret(text: &str) -> Result<[u8; KEY], String> {
    read(text, SECRET, "a key")
}

/// One recipient, as written.
pub fn read_public(text: &str) -> Result<[u8; KEY], String> {
    read(text, PUBLIC, "a recipient")
}

/// Comment lines are skipped, the way a key file people edit needs.
fn read(text: &str, prefix: &str, what: &str) -> Result<[u8; KEY], String> {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .ok_or_else(|| format!("no {what} in it"))?;
    let body = line
        .strip_prefix(prefix)
        .ok_or_else(|| format!("{what} begins with {prefix}, and this does not"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|_| format!("{what} is not base64"))?;
    bytes
        .try_into()
        .map_err(|_| format!("{what} is 32 bytes, and this is not"))
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The secret key a store derives from its profile's.
///
/// **Derived rather than stored**, so there is one secret on the machine instead of two: carrying
/// the profile to another machine carries its stores with it, and there is no second file to keep
/// in step or forget to copy. The store's name is in the salt, so two stores under one profile do
/// not share a key and a file from one cannot be opened with the other's.
pub fn derive_store_key(profile: &[u8; KEY], store: &str) -> Result<[u8; KEY], String> {
    let mut key = [0u8; KEY];
    hkdf::Hkdf::<sha2::Sha256>::new(Some(store.as_bytes()), profile)
        .expand(b"oslo secret store v1", &mut key)
        .map_err(|_| "the store key could not be derived".to_string())?;
    Ok(key)
}

/// A new secret key, from the operating system's randomness and nowhere else.
pub fn generate_secret() -> Result<[u8; KEY], String> {
    random::<KEY>()
}

/// `N` bytes from the operating system.
///
/// **`getrandom`, not a seeded generator.** A nonce, a file key or an ephemeral secret drawn from
/// anything this process could have predicted is the one mistake in this file that no test catches.
fn random<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|e| format!("no randomness from the system: {e}"))?;
    Ok(bytes)
}

#[cfg(test)]
#[path = "native/tests.rs"]
mod tests;
