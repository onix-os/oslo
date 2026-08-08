pub const START: &[u8] = b"\x1b[200~";
pub const END: &[u8] = b"\x1b[201~";
pub const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    TooLarge,
    InvalidUtf8,
}

pub struct Paste {
    bytes: Vec<u8>,
    possible_end: Vec<u8>,
    seen: usize,
    overflowed: bool,
}

impl Paste {
    pub fn new() -> Paste {
        Paste {
            bytes: Vec::new(),
            possible_end: Vec::with_capacity(END.len()),
            seen: 0,
            overflowed: false,
        }
    }

    pub fn push(&mut self, byte: u8) -> bool {
        self.possible_end.push(byte);
        while !END.starts_with(&self.possible_end) {
            let content = self.possible_end.remove(0);
            self.seen += 1;
            if self.seen <= MAX_BYTES {
                self.bytes.push(content);
            } else {
                self.overflowed = true;
            }
        }
        self.possible_end == END
    }

    pub fn finish(self) -> Result<String, Error> {
        if self.overflowed {
            return Err(Error::TooLarge);
        }
        String::from_utf8(self.bytes).map_err(|_| Error::InvalidUtf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_end_prefix_inside_text_is_preserved() {
        let mut paste = Paste::new();
        for byte in b"a\x1b[20xb\x1b[201~" {
            if paste.push(*byte) {
                break;
            }
        }
        assert_eq!(paste.finish(), Ok("a\x1b[20xb".to_string()));
    }

    #[test]
    fn empty_and_escape_filled_pastes_are_preserved() {
        let mut empty = Paste::new();
        for byte in END {
            assert_eq!(empty.push(*byte), *byte == b'~');
        }
        assert_eq!(empty.finish(), Ok(String::new()));

        let mut escaped = Paste::new();
        for byte in b"a\x1b[31mb\x1b[0m\x1b[201~" {
            if escaped.push(*byte) {
                break;
            }
        }
        assert_eq!(escaped.finish(), Ok("a\x1b[31mb\x1b[0m".to_string()));
    }

    #[test]
    fn every_end_marker_split_waits_for_completion() {
        for at in 1..END.len() {
            let mut paste = Paste::new();
            for byte in &END[..at] {
                assert!(!paste.push(*byte), "split {at}");
            }
            for byte in &END[at..] {
                paste.push(*byte);
            }
            assert_eq!(paste.finish(), Ok(String::new()), "split {at}");
        }
    }

    #[test]
    fn invalid_utf8_is_rejected_whole() {
        let mut paste = Paste::new();
        for byte in [0xff].into_iter().chain(END.iter().copied()) {
            if paste.push(byte) {
                break;
            }
        }
        assert_eq!(paste.finish(), Err(Error::InvalidUtf8));
    }

    #[test]
    fn an_oversized_paste_drains_to_its_marker() {
        let mut paste = Paste::new();
        for _ in 0..=MAX_BYTES {
            assert!(!paste.push(b'x'));
        }
        for byte in END {
            paste.push(*byte);
        }
        assert_eq!(paste.finish(), Err(Error::TooLarge));
    }
}
