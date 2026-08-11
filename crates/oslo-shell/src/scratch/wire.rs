//! What travels on the socket.
//!
//! **Asymmetric, on purpose.** Output from the scratch is raw bytes: it is already a stream, nothing
//! needs to be said about it, and a client writes it to the terminal without looking. Input from a
//! client is framed, because it carries one thing that is not keystrokes — the size of the window
//! it is being displayed in, which the keeper has to push onto the pty with `TIOCSWINSZ` or every
//! full-screen program inside the scratch draws for the wrong terminal.
//!
//! ```text
//!   client ──► keeper      0x00 len(u16be) bytes…    the keys you pressed
//!                          0x01 rows(u16be) cols(u16be)   the window changed
//!
//!   keeper ──► client      bytes…                    exactly what the pty said
//! ```
//!
//! Two message types is the whole protocol, and it should stay that way: anything richer belongs in
//! the shell inside the scratch, which is a shell and already has a language.

/// The most a single data frame carries. Bigger reads are simply split.
pub const MAX: usize = 8192;

const DATA: u8 = 0x00;
const RESIZE: u8 = 0x01;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Data(Vec<u8>),
    Resize { rows: u16, cols: u16 },
}

/// Frame `bytes` as one or more data messages.
pub fn data(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 3);
    for chunk in bytes.chunks(MAX) {
        out.push(DATA);
        out.extend_from_slice(&(chunk.len() as u16).to_be_bytes());
        out.extend_from_slice(chunk);
    }
    out
}

/// Frame a window size.
pub fn resize(rows: u16, cols: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(5);
    out.push(RESIZE);
    out.extend_from_slice(&rows.to_be_bytes());
    out.extend_from_slice(&cols.to_be_bytes());
    out
}

/// Pull whole messages off a buffer, leaving any partial one behind.
///
/// **A socket read is not a message.** It can carry half of one, or three and a half, and a decoder
/// that assumed otherwise would work perfectly until the day a keystroke arrived at a buffer
/// boundary. The buffer is drained only as far as the last complete message.
pub fn take(buffer: &mut Vec<u8>) -> Vec<Message> {
    let mut found = Vec::new();
    let mut at = 0;
    while let Some((message, used)) = parse(&buffer[at..]) {
        found.push(message);
        at += used;
    }
    buffer.drain(..at);
    found
}

/// One message from the front of `bytes`, and how much of it was used.
fn parse(bytes: &[u8]) -> Option<(Message, usize)> {
    match *bytes.first()? {
        DATA => {
            let len = u16::from_be_bytes([*bytes.get(1)?, *bytes.get(2)?]) as usize;
            let end = 3 + len;
            (bytes.len() >= end).then(|| (Message::Data(bytes[3..end].to_vec()), end))
        }
        RESIZE => {
            let rows = u16::from_be_bytes([*bytes.get(1)?, *bytes.get(2)?]);
            let cols = u16::from_be_bytes([*bytes.get(3)?, *bytes.get(4)?]);
            Some((Message::Resize { rows, cols }, 5))
        }
        // A byte that names no message means the stream is not one. Saying so is better than
        // guessing, and the caller drops the client.
        _ => None,
    }
}

/// Whether the front of `buffer` can never become a message, however much more arrives.
pub fn corrupt(buffer: &[u8]) -> bool {
    buffer
        .first()
        .is_some_and(|kind| *kind != DATA && *kind != RESIZE)
}

#[cfg(test)]
mod tests;
