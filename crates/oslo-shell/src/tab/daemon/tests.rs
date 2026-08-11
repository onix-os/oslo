use super::*;

#[test]
fn a_request_survives_the_round_trip() {
    for request in [Request::List, Request::Attach("work".to_string())] {
        let bytes = frame(&request);
        let (back, used) = parse(&bytes).expect("must parse");
        assert_eq!(back, request);
        assert_eq!(used, bytes.len(), "the whole frame is consumed");
    }
}

/// A socket read is not a message: it can carry half of one, and a decoder that assumed otherwise
/// would work until the day a name arrived at a buffer boundary.
#[test]
fn half_a_request_is_not_a_request_yet() {
    let whole = frame(&Request::Attach("work".to_string()));
    for cut in 1..whole.len() {
        assert_eq!(parse(&whole[..cut]), None, "cut at {cut}");
        assert!(!unparseable(&whole[..cut]), "cut at {cut} is still coming");
    }
    assert!(parse(&whole).is_some());
}

/// Anything after the request belongs to whoever is spliced onto it, not to the parser.
#[test]
fn what_follows_a_request_is_left_alone() {
    let mut bytes = frame(&Request::Attach("work".to_string()));
    let head = bytes.len();
    bytes.extend_from_slice(b"typed after");
    let (request, used) = parse(&bytes).expect("must parse");
    assert_eq!(request, Request::Attach("work".to_string()));
    assert_eq!(used, head);
}

/// A first byte that names no request never will, however much more arrives — so the daemon drops
/// the client instead of waiting for a message that cannot come.
#[test]
fn a_stream_that_is_not_this_protocol_is_refused() {
    assert!(unparseable(b"GET / HTTP/1.1"));
    assert!(unparseable(&[0x00]));
    assert!(!unparseable(&[LIST]));
    assert!(!unparseable(&[ATTACH]));
    assert!(!unparseable(&[]), "nothing yet is not the same as wrong");
}

#[test]
fn the_socket_sits_beside_the_tabs_it_serves() {
    let (_scratch, _serial) = crate::tab::scratch();
    assert_eq!(socket().parent(), Some(dir::path()).as_deref());
    assert_eq!(
        socket().file_name().and_then(|n| n.to_str()),
        Some("daemon.sock")
    );
}
