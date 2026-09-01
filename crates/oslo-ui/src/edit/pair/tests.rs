use super::*;

/// A buffer with the cursor where `|` is.
fn at(marked: &str) -> Buffer {
    let cursor = marked.find('|').expect("mark the cursor with |");
    let text = marked.replace('|', "");
    let mut buffer = Buffer::from_text(&text);
    buffer.set_cursor(cursor);
    buffer
}

fn typing(marked: &str, typed: char) -> Pairing {
    decide(&at(marked), typed)
}

#[test]
fn an_opener_at_the_end_of_a_line_brings_its_partner() {
    assert_eq!(typing("echo |", '('), Pairing::Close(')'));
    assert_eq!(typing("echo |", '['), Pairing::Close(']'));
    assert_eq!(typing("echo |", '{'), Pairing::Close('}'));
    assert_eq!(typing("echo |", '"'), Pairing::Close('"'));
    assert_eq!(typing("echo |", '\''), Pairing::Close('\''));
    assert_eq!(typing("echo |", '`'), Pairing::Close('`'));
    assert_eq!(typing("|", '('), Pairing::Close(')'));
}

/// **The apostrophe rule.** In a shell a stray quote swallows the rest of the line, so getting this
/// wrong is worse than never pairing at all.
#[test]
fn a_quote_after_a_word_is_an_apostrophe() {
    assert_eq!(typing("it|", '\''), Pairing::Plain);
    assert_eq!(typing("don|", '\''), Pairing::Plain);
    assert_eq!(typing("Bob|", '"'), Pairing::Plain);
    assert_eq!(typing("x1|", '`'), Pairing::Plain);
    assert_eq!(typing("$(x)|", '\''), Pairing::Plain);
    // A space before it, and it is opening something again.
    assert_eq!(typing("it |", '\''), Pairing::Close('\''));
}

/// A bracket is not an apostrophe: `echo(` after a word is still opening a bracket.
#[test]
fn a_bracket_after_a_word_still_pairs() {
    assert_eq!(typing("f|", '('), Pairing::Close(')'));
    assert_eq!(typing("arr|", '['), Pairing::Close(']'));
}

/// Something is already there for the closer to be in the way of.
#[test]
fn nothing_is_opened_in_front_of_a_word() {
    assert_eq!(typing("echo |x", '('), Pairing::Plain);
    assert_eq!(typing("echo |1", '"'), Pairing::Plain);
    assert_eq!(typing("echo |$x", '('), Pairing::Plain);
    assert_eq!(typing("echo |(a)", '('), Pairing::Plain);
    // But whitespace or the end of the line is room enough.
    assert_eq!(typing("echo | x", '('), Pairing::Close(')'));
    assert_eq!(typing("echo |)", '('), Pairing::Close(')'));
}

/// **Stepping over is decided before opening**, or a closing quote would open a new pair every
/// time — the character that closes one is the character that opens one.
#[test]
fn a_closer_already_there_is_stepped_over() {
    assert_eq!(typing("echo (|)", ')'), Pairing::Skip);
    assert_eq!(typing("echo [a|]", ']'), Pairing::Skip);
    assert_eq!(typing("echo \"hi|\"", '"'), Pairing::Skip);
    assert_eq!(typing("echo '|'", '\''), Pairing::Skip);
}

/// A closer with nothing to close is just a character.
#[test]
fn a_closer_on_its_own_is_ordinary() {
    assert_eq!(typing("echo |", ')'), Pairing::Plain);
    assert_eq!(typing("echo a|b", ']'), Pairing::Plain);
}

/// An escaped character is data, not syntax.
#[test]
fn a_backslash_stops_a_pair() {
    assert_eq!(typing("echo \\|", '"'), Pairing::Plain);
    assert_eq!(typing("echo \\|", '('), Pairing::Plain);
}

/// Both halves came from one keystroke, so one backspace takes both.
#[test]
fn backspace_takes_a_pair_it_made() {
    assert!(straddles_a_pair(&at("echo (|)")));
    assert!(straddles_a_pair(&at("echo \"|\"")));
    assert!(straddles_a_pair(&at("echo {|}")));
}

/// Anything that is not still a pair is left alone — deleting a character the user typed
/// themselves is the one thing this must never do.
#[test]
fn backspace_leaves_everything_else_alone() {
    assert!(!straddles_a_pair(&at("echo (a|)")), "no longer adjacent");
    assert!(!straddles_a_pair(&at("echo (\"|")), "not a pair");
    assert!(!straddles_a_pair(&at("echo ()|")), "already typed past it");
    assert!(!straddles_a_pair(&at("|")), "nothing on either side");
    assert!(!straddles_a_pair(&at("echo |")), "nothing to the right");
}

/// Off is off, on both paths.
///
/// **The only test that touches the switch**, and the reason the rules above are a separate
/// function: the flag is process-wide, so a test that flipped it while asking a rule question
/// raced every other test in this file — and did, until they were split.
#[test]
fn nothing_happens_when_it_is_turned_off() {
    set_enabled(false);
    assert_eq!(on_insert(&at("echo |"), '('), Pairing::Plain);
    assert_eq!(on_insert(&at("echo (|)"), ')'), Pairing::Plain);
    assert!(!on_backspace(&at("echo (|)")));
    set_enabled(true);
    assert_eq!(on_insert(&at("echo |"), '('), Pairing::Close(')'));
    assert!(on_backspace(&at("echo (|)")));
}
