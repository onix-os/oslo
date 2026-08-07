#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub button: Button,
    pub column: usize,
    pub row: usize,
    pub pressed: bool,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

pub const ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1006h";
pub const DISABLE: &[u8] = b"\x1b[?1006l\x1b[?1000l";
const CURSOR_POSITION_QUERY: &[u8] = b"\x1b[?6n";

pub fn cursor_position(fd: i32) -> (Option<(usize, usize)>, Vec<u8>) {
    let (reply, pending) =
        super::query::query_sequence(fd, CURSOR_POSITION_QUERY, 60, position_reply);
    (reply.as_deref().and_then(parse_position), pending)
}

fn position_reply(bytes: &[u8]) -> super::query::ReplyMatch {
    use super::query::ReplyMatch;
    if [
        b"\x1b".as_slice(),
        b"\x1b[".as_slice(),
        b"\x1b[?".as_slice(),
    ]
    .contains(&bytes)
    {
        return ReplyMatch::Prefix;
    }
    if !bytes.starts_with(b"\x1b[?") {
        return ReplyMatch::Reject;
    }
    let Some(&last) = bytes.last() else {
        return ReplyMatch::Prefix;
    };
    if last == b'R' {
        return if parse_position(bytes).is_some() {
            ReplyMatch::Complete
        } else {
            ReplyMatch::Reject
        };
    }
    if last.is_ascii_digit() || last == b';' {
        ReplyMatch::Prefix
    } else {
        ReplyMatch::Reject
    }
}

fn parse_position(bytes: &[u8]) -> Option<(usize, usize)> {
    let body = std::str::from_utf8(bytes)
        .ok()?
        .strip_prefix("\x1b[?")?
        .strip_suffix('R')?;
    let (row, column) = body.split_once(';')?;
    Some((
        row.parse::<usize>().ok()?.checked_sub(1)?,
        column.parse::<usize>().ok()?.checked_sub(1)?,
    ))
}

pub fn decode(sequence: &[u8]) -> Option<Event> {
    let text = std::str::from_utf8(sequence).ok()?;
    let pressed = text.ends_with('M');
    if !pressed && !text.ends_with('m') {
        return None;
    }
    let body = text.strip_prefix("\x1b[<")?.strip_suffix(['M', 'm'])?;
    let mut fields = body.split(';');
    let code = fields.next()?.parse::<u8>().ok()?;
    let column = fields.next()?.parse::<usize>().ok()?.checked_sub(1)?;
    let row = fields.next()?.parse::<usize>().ok()?.checked_sub(1)?;
    if fields.next().is_some() || code & 96 != 0 {
        return None;
    }
    Some(Event {
        button: match code & 3 {
            0 => Button::Left,
            1 => Button::Middle,
            2 => Button::Right,
            other => Button::Other(other),
        },
        column,
        row,
        pressed,
        shift: code & 4 != 0,
        alt: code & 8 != 0,
        ctrl: code & 16 != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_clicks_are_structured() {
        assert_eq!(
            decode(b"\x1b[<20;12;7M"),
            Some(Event {
                button: Button::Left,
                column: 11,
                row: 6,
                pressed: true,
                shift: true,
                alt: false,
                ctrl: true,
            })
        );
        assert!(!decode(b"\x1b[<0;12;7m").expect("release").pressed);
    }

    #[test]
    fn cursor_positions_are_zero_based() {
        assert_eq!(CURSOR_POSITION_QUERY, b"\x1b[?6n");
        assert_eq!(parse_position(b"\x1b[?12;4R"), Some((11, 3)));
        assert_eq!(parse_position(b"\x1b[?12;4;1R"), None);
        assert_eq!(
            position_reply(b"\x1b[?12;"),
            crate::ui::term::query::ReplyMatch::Prefix
        );
        assert_eq!(
            position_reply(b"\x1b[?12;4R"),
            crate::ui::term::query::ReplyMatch::Complete
        );
    }
}
