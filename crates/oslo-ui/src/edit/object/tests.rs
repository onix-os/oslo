use super::*;

/// A buffer with the cursor where `|` is.
fn at(marked: &str) -> (Buffer, usize) {
    let cursor = marked.find('|').expect("mark the cursor with |");
    let text = marked.replace('|', "");
    (Buffer::from_text(&text), cursor)
}

/// What the object covers, shown as the text it would take.
fn taken(marked: &str, around: bool, object: char) -> Option<String> {
    let (buffer, cursor) = at(marked);
    let span = find(&buffer, cursor, around, object)?;
    let text: Vec<char> = buffer.text().chars().collect();
    Some(text[span.from..span.to].iter().collect())
}

fn inner(marked: &str, object: char) -> String {
    taken(marked, false, object).unwrap_or_else(|| panic!("no object at {marked}"))
}

fn round(marked: &str, object: char) -> String {
    taken(marked, true, object).unwrap_or_else(|| panic!("no object at {marked}"))
}

#[test]
fn a_word_is_the_run_the_cursor_is_in() {
    assert_eq!(inner("echo he|llo there", 'w'), "hello");
    assert_eq!(inner("echo |hello there", 'w'), "hello");
    assert_eq!(inner("echo hell|o there", 'w'), "hello");
    assert_eq!(inner("|echo hello", 'w'), "echo");
}

/// **The three kinds are what `iw` *is*.** `foo.bar` is three objects to `w` and one to `W`, which
/// is the whole difference between them and why a path wants the capital.
#[test]
fn punctuation_is_its_own_kind_until_the_capital() {
    assert_eq!(inner("src/li|b.rs", 'w'), "lib");
    assert_eq!(inner("src|/lib.rs", 'w'), "/");
    assert_eq!(inner("src/li|b.rs", 'W'), "src/lib.rs");
    assert_eq!(inner("a=|1 b=2", 'W'), "a=1");
}

/// `aw` takes the whitespace that goes with the word, so `daw` leaves one gap and not none.
#[test]
fn around_a_word_takes_the_space_with_it() {
    assert_eq!(round("echo he|llo there", 'w'), "hello ");
    // At the end of the line there is no trailing space, so the leading one goes instead.
    assert_eq!(round("echo the|re", 'w'), " there");
    // On the whitespace itself, `aw` takes the word after it.
    assert_eq!(round("echo| hello", 'w'), " hello");
}

#[test]
fn a_quote_object_is_what_is_between_the_quotes() {
    assert_eq!(inner("echo \"he|llo\"", '"'), "hello");
    assert_eq!(round("echo \"he|llo\"", '"'), "\"hello\"");
    assert_eq!(inner("echo 'a b|c'", '\''), "a bc");
    assert_eq!(inner("echo `da|te`", '`'), "date");
}

/// **Quotes are paired from the start of the line**, because the same character opens and closes:
/// the one to your left is an opener or a closer depending only on how many came before it.
#[test]
fn the_right_pair_of_quotes_is_found() {
    // The cursor is between two pairs — the next one along wins, as in vim.
    assert_eq!(inner("\"one\" |x \"two\"", '"'), "two");
    // Inside the second pair, it is the second pair.
    assert_eq!(inner("\"one\" \"tw|o\"", '"'), "two");
    // An escaped quote does not end the string.
    assert_eq!(inner("echo \"a\\\"b|c\"", '"'), "a\\\"bc");
}

#[test]
fn a_bracket_object_is_the_innermost_pair_around_the_cursor() {
    assert_eq!(inner("f(a, b|, c)", '('), "a, b, c");
    assert_eq!(round("f(a, b|, c)", '('), "(a, b, c)");
    assert_eq!(inner("f(a, g(b|), c)", '('), "b");
    assert_eq!(inner("arr[i|dx]", '['), "idx");
    assert_eq!(inner("${na|me}", '{'), "name");
    // `b` and `B` are vim's aliases for the two reached for most.
    assert_eq!(inner("f(a|b)", 'b'), "ab");
    assert_eq!(inner("{a|b}", 'B'), "ab");
    // Either bracket names the same object.
    assert_eq!(inner("f(a|b)", ')'), "ab");
}

/// Sitting on the opener is being inside it — which is where `f(` leaves the cursor.
#[test]
fn the_cursor_on_a_bracket_is_inside_it() {
    assert_eq!(inner("f|(ab)", '('), "ab");
    assert_eq!(round("f|(ab)", '('), "(ab)");
}

/// Nothing there is `None`, and `None` is how the keymap knows to do nothing at all.
#[test]
fn an_object_that_is_not_there_is_refused() {
    assert!(taken("echo |hello", false, '(').is_none());
    assert!(taken("echo |hello", false, '"').is_none());
    assert!(taken("f(a|b", false, '(').is_none(), "never closed");
    assert!(taken("|", false, 'w').is_none(), "an empty line");
    assert!(
        taken("echo |x", false, 'z').is_none(),
        "not an object at all"
    );
}

/// An empty pair is a real object covering nothing, which deletes nothing and is correct.
#[test]
fn an_empty_pair_is_an_empty_range() {
    assert_eq!(inner("f(|)", '('), "");
    assert_eq!(round("f(|)", '('), "()");
    assert_eq!(inner("echo \"|\"", '"'), "");
}
