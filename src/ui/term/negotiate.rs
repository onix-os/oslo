//! One bounded startup exchange for optional terminal capabilities.

use super::capability::Verified;
use super::query::{self, Background, ReplyMatch};
use std::sync::atomic::{AtomicU64, Ordering};

const TIMEOUT_MS: u64 = 100;
const _: () = assert!(TIMEOUT_MS <= 100);
static QUERY_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Result {
    pub background: Option<Background>,
    pub verified: Verified,
    pub pending: Vec<u8>,
}

pub fn on(fd: i32, query_background: bool) -> Result {
    let id = format!(
        "oslo-{}-{}",
        std::process::id(),
        QUERY_ID.fetch_add(1, Ordering::Relaxed)
    );
    let request = request(&id, query_background);
    let (replies, pending) = query::query_sequences(
        fd,
        &request,
        TIMEOUT_MS,
        |bytes| classify(bytes, &id),
        |replies| replies.iter().any(|reply| is_primary_da(reply)),
    );
    let (background, verified) = select(&replies, &id);
    Result {
        background,
        verified,
        pending,
    }
}

fn request(id: &str, query_background: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if query_background {
        out.extend_from_slice(b"\x1b]11;?\x1b\\");
    }
    out.extend_from_slice(b"\x1b[?u\x1b[?2026$p");
    out.extend_from_slice(format!("\x1b]99;i={id}:p=?;\x1b\\").as_bytes());
    out.extend_from_slice(b"\x1b[c");
    out
}

fn classify(bytes: &[u8], id: &str) -> ReplyMatch {
    if bytes.starts_with(b"\x1b]") || b"\x1b]".starts_with(bytes) {
        return classify_osc(bytes, id);
    }
    if bytes.starts_with(b"\x1b[") || b"\x1b[".starts_with(bytes) {
        return classify_csi(bytes);
    }
    ReplyMatch::Reject
}

fn classify_osc(bytes: &[u8], id: &str) -> ReplyMatch {
    let osc11 = b"\x1b]11;rgb:";
    let osc99 = format!("\x1b]99;i={id}");
    let accepted = osc11.starts_with(bytes)
        || osc99.as_bytes().starts_with(bytes)
        || bytes.starts_with(osc11)
        || bytes.starts_with(osc99.as_bytes());
    if !accepted {
        return ReplyMatch::Reject;
    }
    if bytes.ends_with(b"\x07") || bytes.ends_with(b"\x1b\\") {
        ReplyMatch::Complete
    } else {
        ReplyMatch::Prefix
    }
}

fn classify_csi(bytes: &[u8]) -> ReplyMatch {
    if b"\x1b[".starts_with(bytes) {
        return ReplyMatch::Prefix;
    }
    if !bytes.starts_with(b"\x1b[") {
        return ReplyMatch::Reject;
    }
    let Some(&last) = bytes.last() else {
        return ReplyMatch::Prefix;
    };
    if matches!(last, b'u' | b'c') && bytes.starts_with(b"\x1b[?") {
        return ReplyMatch::Complete;
    }
    if last == b'y' && bytes.starts_with(b"\x1b[?2026;") && bytes.contains(&b'$') {
        return ReplyMatch::Complete;
    }
    if last.is_ascii_digit() || matches!(last, b'?' | b';' | b'$') {
        ReplyMatch::Prefix
    } else {
        ReplyMatch::Reject
    }
}

fn select(replies: &[Vec<u8>], id: &str) -> (Option<Background>, Verified) {
    let mut background = None;
    let mut verified = Verified::default();
    let Some(barrier) = replies.iter().position(|reply| is_primary_da(reply)) else {
        return (None, verified);
    };
    let mut sync = None;
    let mut sync_contradiction = false;
    for reply in &replies[..barrier] {
        if reply.starts_with(b"\x1b]11;") {
            background = std::str::from_utf8(reply)
                .ok()
                .and_then(query::parse_background);
        } else if reply.ends_with(b"u") && reply.starts_with(b"\x1b[?") {
            verified.kitty_keyboard = true;
        } else if let Some(state) = sync_state(reply) {
            let supported = matches!(state, 1 | 2);
            sync_contradiction |= sync.is_some_and(|previous| previous != supported);
            sync = Some(supported);
        } else if let Some(unfocused) = osc99_support(reply, id) {
            verified.osc99_notifications = true;
            verified.osc99_unfocused = unfocused;
        }
    }
    verified.synchronized_output = sync == Some(true) && !sync_contradiction;
    (background, verified)
}

fn is_primary_da(reply: &[u8]) -> bool {
    reply.starts_with(b"\x1b[?") && reply.ends_with(b"c")
}

fn sync_state(reply: &[u8]) -> Option<u8> {
    let body = std::str::from_utf8(reply)
        .ok()?
        .strip_prefix("\x1b[?2026;")?
        .strip_suffix("$y")?;
    body.parse().ok()
}

fn osc99_support(reply: &[u8], id: &str) -> Option<bool> {
    let reply = std::str::from_utf8(reply).ok()?;
    let contents = reply
        .strip_prefix("\x1b]99;")?
        .strip_suffix('\u{7}')
        .or_else(|| reply.strip_prefix("\x1b]99;")?.strip_suffix("\x1b\\"))?;
    let (metadata, support) = contents.split_once(';')?;
    let mut matching_id = false;
    let mut support_query = false;
    for field in metadata.split(':') {
        let Some((name, value)) = field.split_once('=') else {
            continue;
        };
        match name {
            "i" => matching_id = value == id,
            "p" => support_query = value == "?",
            _ => {}
        }
    }
    if !matching_id || !support_query {
        return None;
    }

    let mut payloads = None;
    let mut occasions = "";
    for field in support.split(':') {
        let Some((name, value)) = field.split_once('=') else {
            continue;
        };
        match name {
            "p" => payloads = Some(value),
            "o" => occasions = value,
            _ => {}
        }
    }
    let payloads = payloads?;
    let title = payloads.split(',').any(|value| value == "title");
    let body = payloads.split(',').any(|value| value == "body");
    (title && body).then(|| occasions.split(',').any(|occasion| occasion == "unfocused"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_batch_has_one_final_device_attributes_barrier() {
        let request = request("oslo-test", true);
        assert!(request.starts_with(b"\x1b]11;?\x1b\\\x1b[?u\x1b[?2026$p"));
        assert!(request.windows(9).any(|part| part == b"i=oslo-te"));
        assert!(request.ends_with(b"\x1b[c"));
        assert_eq!(
            request.windows(3).filter(|part| *part == b"\x1b[c").count(),
            1
        );
    }

    #[test]
    fn replies_before_the_barrier_select_only_proven_features() {
        let id = "oslo-test";
        let replies = vec![
            b"\x1b[?1u".to_vec(),
            b"\x1b[?2026;2$y".to_vec(),
            format!("\x1b]99;i={id}:p=?;p=title,body:o=unfocused\x1b\\").into_bytes(),
            b"\x1b[?62;4c".to_vec(),
            b"\x1b[?2026;1$y".to_vec(),
        ];
        let (_, verified) = select(&replies, id);
        assert!(verified.kitty_keyboard);
        assert!(verified.synchronized_output);
        assert!(verified.osc99_notifications);
        assert!(verified.osc99_unfocused);
    }

    #[test]
    fn conservative_sync_states_and_correlated_notifications() {
        for state in [0, 3, 4] {
            let (_, verified) = select(
                &[
                    format!("\x1b[?2026;{state}$y").into_bytes(),
                    b"\x1b[?1c".to_vec(),
                ],
                "id",
            );
            assert!(!verified.synchronized_output, "state {state}");
        }
        assert_eq!(
            osc99_support(b"\x1b]99;i=other:p=?;p=title,body\x07", "id"),
            None
        );
        assert_eq!(
            osc99_support(b"\x1b]99;i=id:p=?;p=title,body\x07", "id"),
            Some(false)
        );
        assert_eq!(
            osc99_support(b"\x1b]99;i=id:p=title;title,body\x07", "id"),
            None
        );
        let (_, absent_barrier) = select(&[b"\x1b[?1u".to_vec()], "id");
        assert_eq!(absent_barrier, Verified::default());
        let (_, contradictory) = select(
            &[
                b"\x1b[?2026;1$y".to_vec(),
                b"\x1b[?2026;4$y".to_vec(),
                b"\x1b[?1c".to_vec(),
            ],
            "id",
        );
        assert!(!contradictory.synchronized_output);
    }

    #[test]
    fn every_reply_prefix_waits_and_unrelated_bytes_are_rejected() {
        let replies = [
            b"\x1b]11;rgb:00/00/00\x1b\\".as_slice(),
            b"\x1b[?1u".as_slice(),
            b"\x1b[?2026;1$y".as_slice(),
            b"\x1b]99;i=id:p=?;p=title,body\x07".as_slice(),
            b"\x1b[?62;4c".as_slice(),
        ];
        for reply in replies {
            for at in 1..reply.len() {
                assert_eq!(
                    classify(&reply[..at], "id"),
                    ReplyMatch::Prefix,
                    "{reply:?} at {at}"
                );
            }
            assert_eq!(classify(reply, "id"), ReplyMatch::Complete);
        }
        assert_eq!(classify(b"typed", "id"), ReplyMatch::Reject);
    }

    #[test]
    fn optional_replies_are_selected_independently() {
        let id = "oslo-test";
        let optional = [
            b"\x1b[?1u".to_vec(),
            b"\x1b[?2026;1$y".to_vec(),
            format!("\x1b]99;i={id}:p=?;p=title,body\x1b\\").into_bytes(),
        ];
        for absent in 0..optional.len() {
            let mut replies: Vec<_> = optional
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != absent)
                .map(|(_, reply)| reply.clone())
                .collect();
            replies.push(b"\x1b[?62;4c".to_vec());
            let (_, verified) = select(&replies, id);
            assert_eq!(verified.kitty_keyboard, absent != 0);
            assert_eq!(verified.synchronized_output, absent != 1);
            assert_eq!(verified.osc99_notifications, absent != 2);
        }
    }

    #[test]
    fn typing_between_batch_replies_is_preserved_in_order() {
        let id = "oslo-test";
        let bytes = [
            b"x".as_slice(),
            b"\x1b[?1u".as_slice(),
            b"y".as_slice(),
            b"\x1b[?2026;1$y".as_slice(),
            b"z".as_slice(),
            b"\x1b[?62;4c".as_slice(),
        ]
        .concat();
        let mut broker = query::Broker::new(|bytes| classify(bytes, id), 4096);
        let mut replies = Vec::new();
        for byte in bytes {
            if let Some(reply) = broker.push(byte) {
                replies.push(reply);
            }
        }
        assert_eq!(
            replies,
            [
                b"\x1b[?1u".to_vec(),
                b"\x1b[?2026;1$y".to_vec(),
                b"\x1b[?62;4c".to_vec(),
            ]
        );
        assert_eq!(broker.finish(), b"xyz");
    }

    #[test]
    fn malformed_and_oversized_notification_replies_do_not_enable_support() {
        let id = "oslo-test";
        let (_, malformed) = select(
            &[
                format!("\x1b]99;i={id}:p=?;p=title\x1b\\").into_bytes(),
                b"\x1b[?62;4c".to_vec(),
            ],
            id,
        );
        assert!(!malformed.osc99_notifications);

        let mut oversized = format!("\x1b]99;i={id}:p=?;p=title,body:").into_bytes();
        oversized.extend(std::iter::repeat_n(b'x', 4096));
        oversized.extend_from_slice(b"\x1b\\");
        let mut broker = query::Broker::new(|bytes| classify(bytes, id), 4096);
        assert!(
            oversized
                .into_iter()
                .all(|byte| broker.push(byte).is_none())
        );
    }
}
