//! `oslo.hash`, `oslo.hex` and `oslo.base64` — summarising bytes, and moving them through text.
//!
//! ```lua
//! oslo.hash.sha256("hello")              -- "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
//! oslo.hash.file("/usr/bin/oslo")        -- streamed; the file is never held in memory
//! oslo.hex.encode(oslo.fs.read("k.bin")) -- bytes as text, for somewhere that only takes text
//! oslo.base64.decode(token)              -- and back
//! ```
//!
//! # Why now, and why together
//!
//! These are the calls that only make sense once a shell value can *hold* arbitrary bytes. Before
//! that, `oslo.hash.sha256(oslo.fs.read(p))` would have hashed a lossy rendering of the file rather
//! than the file — an answer that looks like a checksum, matches nothing, and gives no sign of why.
//! Encoding is the same story from the other end: the reason to reach for hex or base64 is to carry
//! bytes somewhere that only accepts text, which needs the bytes to have survived getting here.
//!
//! `encode`/`decode` rather than `hex`/`unhex`, because `oslo.json` already established the pair
//! and one spelling for one idea is worth more than a shorter name.
//!
//! # Hex and base64 are not a hash
//!
//! Worth saying plainly, because they are neighbours here: `oslo.base64.encode` hides nothing and
//! protects nothing. `oslo.secret` is what encrypts. This is a change of alphabet.

use super::util::{failed, ok, opt_text, put, raw, text};
use oslo_base::value::{LuaError, Table, Value};
use sha2::{Digest, Sha256, Sha512};

/// Build `oslo.hash`.
pub fn hash() -> Value {
    let mut it = Table::new();

    // oslo.hash.sha256(bytes) -> its digest, as lower-case hex
    put(&mut it, "sha256", |_, args| {
        let subject = raw(&args, 1, "oslo.hash.sha256")?;
        ok(Value::str(hex_of(&Sha256::digest(&subject))))
    });

    put(&mut it, "sha512", |_, args| {
        let subject = raw(&args, 1, "oslo.hash.sha512")?;
        ok(Value::str(hex_of(&Sha512::digest(&subject))))
    });

    // oslo.hash.file(path, algorithm) -> its digest, or nil + message
    //
    // **Streamed, unlike hashing what `oslo.fs.read` answered.** A checksum is the one thing people
    // reach for on files far too large to hold, and reading a 4 GB image into a Lua string to hash
    // it would be a worse way to answer the same question.
    put(&mut it, "file", |_, args| {
        let path = text(&args, 1, "oslo.hash.file")?;
        let algorithm = opt_text(&args, 2, "oslo.hash.file")?.unwrap_or_else(|| "sha256".into());
        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(e) => return super::util::failed_path(&path, &e),
        };
        match algorithm.as_str() {
            "sha256" => stream(&mut file, Sha256::new(), &path),
            "sha512" => stream(&mut file, Sha512::new(), &path),
            other => Err(LuaError::new(format!(
                "oslo.hash.file: {other:?} is not an algorithm; they are sha256 and sha512"
            ))),
        }
    });

    Value::table(it)
}

/// Read `file` to the end through `digest`, holding one buffer rather than the file.
fn stream<D: Digest + std::io::Write>(
    file: &mut std::fs::File,
    mut digest: D,
    path: &str,
) -> oslo_base::value::LuaResult<Vec<Value>> {
    match std::io::copy(file, &mut digest) {
        Ok(_) => ok(Value::str(hex_of(&digest.finalize()))),
        Err(e) => failed(path, e),
    }
}

/// Build `oslo.hex`.
pub fn hex() -> Value {
    let mut it = Table::new();

    put(&mut it, "encode", |_, args| {
        let subject = raw(&args, 1, "oslo.hex.encode")?;
        ok(Value::str(hex_of(&subject)))
    });

    // oslo.hex.decode(text) -> the bytes, or nil + message
    //
    // Answers rather than raises: text that is not hex is a condition — it came from a file, a
    // command or somebody typing — and not a bug in the script asking.
    put(&mut it, "decode", |_, args| {
        let subject = text(&args, 1, "oslo.hex.decode")?;
        match bytes_of_hex(&subject) {
            Some(bytes) => ok(Value::bytes(&bytes)),
            None => Ok(vec![
                Value::Nil,
                super::problem::new(
                    "oslo.hex.decode: not hexadecimal".to_string(),
                    vec![("kind", Value::str("invalid"))],
                ),
            ]),
        }
    });

    Value::table(it)
}

/// Build `oslo.base64`.
pub fn base64() -> Value {
    let mut it = Table::new();

    put(&mut it, "encode", |_, args| {
        let subject = raw(&args, 1, "oslo.base64.encode")?;
        ok(Value::str(base64_of(&subject)))
    });

    put(&mut it, "decode", |_, args| {
        let subject = text(&args, 1, "oslo.base64.decode")?;
        match bytes_of_base64(&subject) {
            Some(bytes) => ok(Value::bytes(&bytes)),
            None => Ok(vec![
                Value::Nil,
                super::problem::new(
                    "oslo.base64.decode: not base64".to_string(),
                    vec![("kind", Value::str("invalid"))],
                ),
            ]),
        }
    });

    Value::table(it)
}

/// Bytes as lower-case hex.
fn hex_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Hex as bytes, or `None` when it is not hex.
///
/// Upper and lower case both, because a checksum copied off a web page may be either. An odd number
/// of digits is refused rather than padded: `"abc"` could mean `0a bc` or `ab c0`, and guessing is
/// how a checksum quietly stops matching.
fn bytes_of_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let digits: Vec<u8> = text.bytes().collect();
    digits
        .chunks(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

/// The standard alphabet, as RFC 4648 gives it.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Bytes as base64, padded.
fn base64_of(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let packed = (group[0] as u32) << 16
            | (*group.get(1).unwrap_or(&0) as u32) << 8
            | *group.get(2).unwrap_or(&0) as u32;
        for slot in 0..4 {
            // A group of one byte carries two characters, of two bytes three; the rest is padding,
            // which is what makes the length recoverable.
            if slot <= group.len() {
                out.push(ALPHABET[(packed >> (18 - slot * 6) & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Base64 as bytes, or `None` when it is not base64.
///
/// Whitespace is skipped, because base64 arrives wrapped at 64 or 76 columns as often as not — from
/// a PEM file, from `base64` itself, from an email header. Padding is accepted and not required.
fn bytes_of_base64(text: &str) -> Option<Vec<u8>> {
    let mut sextets: Vec<u8> = Vec::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => continue,
            b'=' => break,
            _ => sextets.push(ALPHABET.iter().position(|c| *c == byte)? as u8),
        }
    }
    // One leftover sextet is six bits, which is not a byte and cannot be part of one.
    if sextets.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(sextets.len() / 4 * 3);
    for group in sextets.chunks(4) {
        let mut packed: u32 = 0;
        for (slot, sextet) in group.iter().enumerate() {
            packed |= (*sextet as u32) << (18 - slot * 6);
        }
        for slot in 0..group.len() - 1 {
            out.push((packed >> (16 - slot * 8) & 0xff) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
#[path = "digest/tests.rs"]
mod tests;
