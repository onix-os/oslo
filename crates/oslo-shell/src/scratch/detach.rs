//! The chord that steps out of a scratch, in both of the encodings a terminal may send it in.
//!
//! # Why one byte is not enough
//!
//! A scratch hands a pty to a shell, and that shell asks the terminal for the Kitty keyboard protocol —
//! `\x1b[>5u`, so it can tell `^I` from Scratch in its line editor. That request is not addressed to the
//! client: it is output, so it travels out through the client to the real terminal like any other
//! bytes. From then on the terminal is entitled to send chords the Kitty way, and `^G` arrives as
//! `\x1b[103;5u` rather than as `0x07`.
//!
//! So the shell inside a scratch quietly changes how the key that leaves the scratch is spelled. A client
//! watching for a single byte works right up until the first prompt is drawn and then never fires
//! again — which is exactly as confusing as it sounds, because nothing looks broken.
//!
//! ```text
//!   legacy     ^G  ──►  07
//!   kitty      ^G  ──►  1b 5b 31 30 33 3b 35 75        "\x1b[103;5u"
//!                        ↑     ↑        ↑              CSI  key=103 ('g')  mods=5 (ctrl)
//! ```
//!
//! Both are matched. The terminal decides which to send and is not obliged to say so.

/// `^\`, and the default because nothing in common use binds it. `^X` is nano's Exit and emacs'
/// prefix; a proxy that swallowed it would break both inside every scratch.
pub const DEFAULT: u8 = 0x1c;

/// The detach chord: the byte it is in a plain terminal, and the key it stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    byte: u8,
    code: u32,
}

impl Key {
    /// The chord a control byte stands for.
    ///
    /// The control bytes are a lossy spelling of a chord — `0x07` is `^G` and nothing else — so the
    /// key is recovered rather than carried alongside.
    pub fn from_byte(byte: u8) -> Key {
        let code = match byte {
            0x00 => u32::from(b' '),
            0x01..=0x1a => u32::from(b'a' + byte - 1),
            0x1c..=0x1f => u32::from(b"\\]^_"[(byte - 0x1c) as usize]),
            other => u32::from(other),
        };
        Key { byte, code }
    }

    /// The chord a config wrote by name, or the default if it named something that cannot be one.
    ///
    /// Only the control chords are a single byte, which is the whole set worth binding here: a
    /// detach on `f4` would have to match an escape sequence, and a sequence a program inside could
    /// also send is not one to swallow.
    pub fn named(name: &str) -> Key {
        let name = name.trim().to_ascii_lowercase();
        let letter = name.strip_prefix("ctrl-").and_then(|rest| {
            let mut chars = rest.chars();
            let only = chars.next()?;
            chars.next().is_none().then_some(only)
        });
        match letter {
            Some(letter) if letter.is_ascii() => Key::from_byte((letter as u8) & 0x1f),
            _ => Key::from_byte(DEFAULT),
        }
    }

    /// The byte this chord is, for anything that still needs to speak of it as one.
    pub fn byte(&self) -> u8 {
        self.byte
    }

    /// Where this chord starts in `bytes`, if it is in there at all.
    ///
    /// The offset rather than a yes: everything typed before the chord in the same read is still
    /// the shell's, so a paste that happens to contain it does not lose its first half.
    pub fn find(&self, bytes: &[u8]) -> Option<usize> {
        (0..bytes.len()).find(|at| {
            bytes[*at] == self.byte
                || csi_u(&bytes[*at..]).is_some_and(|(code, mods)| code == self.code && ctrl(mods))
        })
    }
}

/// The key and modifiers of a Kitty `CSI u` sequence at the front of `bytes`.
///
/// Parsed as a CSI sequence proper — parameter bytes until a final byte — rather than by looking
/// for a `u`, so that an unrelated sequence with a `u` somewhere in it is not read as a chord.
fn csi_u(bytes: &[u8]) -> Option<(u32, u8)> {
    if bytes.first()? != &0x1b || bytes.get(1)? != &b'[' {
        return None;
    }
    let mut end = 2;
    while (0x30..=0x3f).contains(bytes.get(end)?) {
        end += 1;
    }
    if bytes[end] != b'u' {
        return None;
    }

    // `key:shifted:base ; mods:event`. Only the first of each section is the chord itself; the rest
    // is what the alternate-key flag adds and is no business of a detach key.
    let mut sections = bytes[2..end].split(|byte| *byte == b';');
    let code = number(sections.next()?)?;
    let mods = sections.next().map_or(Some(1), number)?;
    Some((code, mods as u8))
}

/// The leading number of a `:`-separated section.
fn number(section: &[u8]) -> Option<u32> {
    let first = section.split(|byte| *byte == b':').next()?;
    if first.is_empty() || !first.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(first).ok()?.parse().ok()
}

/// Whether a Kitty modifier field has Ctrl in it. The field is a bitmask plus one, so that an
/// unmodified key can still be reported as `1`.
fn ctrl(mods: u8) -> bool {
    mods.saturating_sub(1) & 4 != 0
}

#[cfg(test)]
mod tests;
