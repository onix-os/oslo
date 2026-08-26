
/// **A tilde after an unquoted `=` or `:` is a tilde prefix too.** POSIX names it for assignment
/// words and bash applies it to any word; without it `export p=~/x` exported a literal `~`, and
/// `PATH=$PATH:~/bin` put a tilde on the path.
#[test]
fn a_tilde_opens_after_an_equals_or_a_colon() {
    let tilde_at = |source: &str| {
        let word = super::quoting::parse_word_source(source).expect("lexes");
        word.parts
            .iter()
            .position(|part| matches!(part, WordPart::Tilde(_)))
    };
    assert_eq!(tilde_at("~/x"), Some(0), "the plain case still works");
    assert_eq!(tilde_at("a=~/x"), Some(1), "after an `=`");
    assert_eq!(tilde_at("a:~/x"), Some(1), "after a `:`");
    assert_eq!(tilde_at("a=b:~/x"), Some(1), "after the `:` of a value");
    // …and nowhere else: a tilde in the middle of a word is an ordinary character.
    assert_eq!(tilde_at("ab~/x"), None);
    assert_eq!(tilde_at("a-~/x"), None);
}

/// **The text before the tilde keeps its place.** Every other branch of the scanner flushes the
/// literal it has collected before pushing a part; this one never had to, because it only fired on
/// an empty buffer. `a=~/x` came back as `/home/youa=/x` for exactly one commit.
#[test]
fn the_text_before_a_tilde_stays_in_front_of_it() {
    let word = super::quoting::parse_word_source("a=~/x").expect("lexes");
    assert!(
        matches!(word.parts.first(), Some(WordPart::Literal(text)) if text == "a="),
        "{:?}",
        word.parts
    );
}
