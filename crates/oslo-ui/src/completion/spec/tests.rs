use super::*;

fn word<'a>(line: &'a str, stem: &str) -> Word<'a> {
    Word {
        start: 0,
        text: line,
        stem: stem.to_string(),
        quote: Quote::None,
        command_position: false,
        prior_words: Vec::new(),
        carried: 0,
        prefix: "",
    }
}

/// **The stem is cut, not merely offset.** `(,)` on `one,src/ma` completes `src/ma`, and a
/// stem left whole would send the path builder looking for a directory called `one,src`.
#[test]
fn retargeting_cuts_the_word_and_moves_where_it_is_written() {
    let whole = word("one,src/ma", "one,src/ma");
    let piece = retarget(&whole, 4).expect("a plain word can be cut");
    assert_eq!(piece.stem, "src/ma");
    assert_eq!(piece.text, "src/ma");
    assert_eq!(piece.start, 4);
    assert_eq!(piece.carried, 0);
}

#[test]
fn a_word_that_is_not_its_own_text_is_left_alone() {
    let escaped = Word {
        text: r"one,my\ file",
        ..word(r"one,my\ file", "one,my file")
    };
    assert!(retarget(&escaped, 4).is_none());
    let quoted = Word {
        quote: Quote::Double,
        ..word("one,x", "one,x")
    };
    assert!(retarget(&quoted, 4).is_none());
}

#[test]
fn nothing_to_cut_gives_the_word_back() {
    let whole = word("plain", "plain");
    assert_eq!(retarget(&whole, 0).map(|w| w.stem), Some("plain".into()));
}

/// A position past the end of what was declared falls to the one declared for every other.
#[test]
fn positions_past_the_declared_ones_use_the_catch_all() {
    let declared = vec![Action::list(["first"]), Action::list(["second"])];
    let any = Action::list(["rest"]);
    assert!(matches!(position(&declared, &any, 0), Action::List(l) if l == &["first"]));
    assert!(matches!(position(&declared, &any, 2), Action::List(l) if l == &["rest"]));
    // …and with nothing declared for every other, nothing at all.
    assert!(position(&declared, &Action::None, 9).is_none());
}
