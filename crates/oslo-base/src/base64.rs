//! Base64, RFC 4648, standard alphabet.
//!
//! **One copy, because there were three.** `marks` had one for `OSC 52`, `term::metadata` had one
//! for `OSC 1337` and `OSC 99`, and `oslo.hash.base64` had a third — all the same twenty lines,
//! and each of them a place a padding bug could live on its own.
//!
//! Written out rather than pulled in: the `base64` crate is in the tree, but only behind
//! `secrets`, and everything here runs in a build that does not have it. Twenty lines is not a
//! dependency to audit, keep current and explain.

/// The standard alphabet, as RFC 4648 gives it.
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Bytes as base64, padded.
pub fn encode(bytes: &[u8]) -> String {
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
pub fn decode(text: &str) -> Option<Vec<u8>> {
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
mod tests {
    use super::{decode, encode};

    /// The RFC 4648 vectors, because a base64 that is wrong by one pad character puts silently
    /// corrupted text on the clipboard — which is worse than putting none there.
    #[test]
    fn the_rfc_vectors_encode_as_published() {
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
        // Bytes that are not text at all still encode.
        assert_eq!(encode(&[0xff, 0x00, 0xff]), "/wD/");
    }

    #[test]
    fn what_was_encoded_decodes_back() {
        for text in ["", "f", "fo", "foo", "foob", "fooba", "foobar"] {
            assert_eq!(
                decode(&encode(text.as_bytes())).as_deref(),
                Some(text.as_bytes())
            );
        }
        // Wrapped at 76 columns, the way `base64` itself writes it.
        assert_eq!(decode("Zm9v\nYmFy").as_deref(), Some(&b"foobar"[..]));
        // Six bits left over is not a byte and cannot be part of one.
        assert_eq!(decode("Zm9vY"), None);
        assert_eq!(decode("not base64!"), None);
    }
}
