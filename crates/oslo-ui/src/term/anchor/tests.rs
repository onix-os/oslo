//! Reading the answer, and refusing everything that is not one.

use super::*;
use crate::term::query::ReplyMatch;

/// **Zero is an answer, not a position.** A terminal that knows the question but has no mark to
/// measure from — the prompt after a `clear` is the ordinary case — says so, and the caller keeps
/// the number it counted rather than treating the top of the block as the cursor's own row.
#[test]
fn the_reply_is_one_based_so_that_zero_can_mean_unmarked() {
    assert_eq!(parse_anchor(b"\x1b[?1440;1n"), Some(0));
    assert_eq!(parse_anchor(b"\x1b[?1440;6n"), Some(5));
    assert_eq!(parse_anchor(b"\x1b[?1440;0n"), None);
}

/// **Everything that is not the reply is somebody's keystroke.** This runs between a command
/// ending and the next prompt, on a terminal that may be answering something else entirely — so a
/// near miss has to be rejected rather than swallowed, and a real prefix has to be waited out.
#[test]
fn only_the_exact_shape_is_taken_for_an_answer() {
    assert_eq!(anchor_reply(b"\x1b"), ReplyMatch::Prefix);
    assert_eq!(anchor_reply(b"\x1b[?1440;"), ReplyMatch::Prefix);
    assert_eq!(anchor_reply(b"\x1b[?1440;1"), ReplyMatch::Prefix);
    assert_eq!(anchor_reply(b"\x1b[?1440;12n"), ReplyMatch::Complete);

    // A cursor position report, which is the reply this most resembles and must not be eaten.
    assert_eq!(anchor_reply(b"\x1b[?12;4R"), ReplyMatch::Reject);
    assert_eq!(anchor_reply(b"x"), ReplyMatch::Reject);

    assert_eq!(QUERY, b"\x1b[?1440n");
}
