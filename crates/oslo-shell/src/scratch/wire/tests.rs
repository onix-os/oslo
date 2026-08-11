use super::*;

#[test]
fn data_round_trips() {
    let mut buffer = data(b"hello");
    assert_eq!(take(&mut buffer), [Message::Data(b"hello".to_vec())]);
    assert!(buffer.is_empty(), "a whole message leaves nothing behind");
}

#[test]
fn a_resize_round_trips() {
    let mut buffer = resize(24, 80);
    assert_eq!(take(&mut buffer), [Message::Resize { rows: 24, cols: 80 }]);
}

/// Several messages in one read, which is what a burst of typing looks like.
#[test]
fn many_messages_in_one_read() {
    let mut buffer = data(b"a");
    buffer.extend(resize(10, 20));
    buffer.extend(data(b"bc"));
    assert_eq!(
        take(&mut buffer),
        [
            Message::Data(b"a".to_vec()),
            Message::Resize { rows: 10, cols: 20 },
            Message::Data(b"bc".to_vec()),
        ]
    );
}

/// **The case a naive decoder gets wrong**: a message split across two reads. Nothing is returned
/// until it is whole, and the half that arrived is kept.
#[test]
fn a_split_message_waits_for_the_rest() {
    let whole = data(b"hello world");
    let (first, second) = whole.split_at(6);

    let mut buffer = first.to_vec();
    assert!(take(&mut buffer).is_empty(), "half a message is no message");
    assert_eq!(buffer.len(), 6, "and it is kept");

    buffer.extend_from_slice(second);
    assert_eq!(take(&mut buffer), [Message::Data(b"hello world".to_vec())]);
    assert!(buffer.is_empty());
}

/// A header can be split too, which is the same bug one byte earlier.
#[test]
fn a_split_header_waits_as_well() {
    let whole = data(b"xy");
    let mut buffer = whole[..2].to_vec();
    assert!(take(&mut buffer).is_empty());
    buffer.extend_from_slice(&whole[2..]);
    assert_eq!(take(&mut buffer), [Message::Data(b"xy".to_vec())]);
}

/// Anything bigger than a frame is split rather than refused.
#[test]
fn a_big_write_becomes_several_frames() {
    let big = vec![b'z'; MAX * 2 + 5];
    let mut buffer = data(&big);
    let messages = take(&mut buffer);
    assert_eq!(messages.len(), 3);
    let rejoined: Vec<u8> = messages
        .into_iter()
        .flat_map(|m| match m {
            Message::Data(bytes) => bytes,
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(rejoined, big, "and it rejoins to exactly what went in");
}

/// A stream that is not this protocol is recognised as such rather than silently ignored.
#[test]
fn nonsense_is_named_as_nonsense() {
    assert!(corrupt(b"\x7fgarbage"));
    assert!(!corrupt(&data(b"fine")));
    assert!(!corrupt(&resize(1, 1)));
    assert!(!corrupt(b""), "nothing yet is not corruption");
}

#[test]
fn an_empty_write_frames_nothing() {
    assert!(data(b"").is_empty());
}
