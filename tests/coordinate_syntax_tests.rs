//! What is a coordinate and what is not — the syntax rules, and the collisions they resolve.
//!
//! `{4}` is line 4 and also a regex repeat count. `{1..3}` is a range of lines and also a brace
//! sequence. `{0:0}` in double quotes is a value and in single quotes is text. None of that can be
//! settled by trying it and seeing; the rules live here, with the failures each one was written
//! against.

mod common;

#[path = "coordinates/fixture.rs"]
mod fixture;

use fixture::shell;
/// **A brace that is not a coordinate keeps its own meaning**, and the two collide in real syntax.
///
/// `{4}` is a coordinate — line 4 — and it is also a regex repeat count. `{1..3}` is a coordinate
/// range and also a brace sequence. Both parse as coordinates perfectly well, so the rule cannot be
/// "try it and see": brace expansion runs on a word's source text *before* the lexer, so an
/// ordinary command word has already become its several words by the time there is a tree to
/// rewrite. Whatever still holds a literal brace is somewhere bash refused to expand one.
#[test]
fn a_brace_form_that_is_not_a_coordinate_is_left_alone() {
    // An assignment's right-hand side is text in bash. Rewriting it emptied it.
    assert_eq!(shell(r#"w=x{1..3}; echo "$w""#), "x{1..3}");
    assert_eq!(shell(r#"w={4}; echo "$w""#), "{4}");
    assert_eq!(
        shell(r#"a=(x{1,2} {3..4}); printf '%s\n' "${a[@]}""#),
        "x1\nx2\n3\n4"
    );
    // A command word still brace-expands, because that ran before any of this.
    assert_eq!(shell("echo {1..3}"), "1 2 3");
    assert_eq!(shell("echo {a,b}"), "a b");
}

/// **The one place this departs from bash on purpose**, written down so it stays a decision.
///
/// bash leaves a one-item group like `{5}` alone, so unlike `{1..3}` it is still a literal brace
/// when the tree is walked — and a one-item group is exactly the shape of a one-dimension
/// coordinate. oslo reads it as line 5, which is the whole point of `{0}` meaning the first line.
///
/// The cost is that with nothing captured it reads empty where bash would have echoed the text
/// back. That is the same answer `{0:0}` gives with nothing captured, so it is at least consistent
/// with itself — and unlike the regex and scalar-assignment cases, nothing else in the shell wanted
/// those characters.
#[test]
fn a_one_item_group_is_a_coordinate_not_a_literal() {
    assert_eq!(
        shell("cat hosts.txt | echo [{1}]"),
        "[web-02  10.0.0.2  apache]"
    );
    // With no stream behind it, empty — where bash would print `{5}`.
    assert_eq!(shell("echo [{5}]"), "[]");
}

/// **A regex owns `{}`**, so the right operand of `=~` never reads as a coordinate.
///
/// The failure this guards was silent and said *yes*: the quantifier was resolved against no
/// stream, `^[0-9]{4}` became `^[0-9]`, and a two-digit string matched a four-digit pattern.
#[test]
fn a_regex_quantifier_survives_the_match() {
    assert_eq!(shell("[[ 20 =~ ^[0-9]{4} ]] && echo yes || echo no"), "no");
    assert_eq!(
        shell("[[ 2024 =~ ^[0-9]{4} ]] && echo yes || echo no"),
        "yes"
    );
    assert_eq!(shell("[[ a =~ ^a{3} ]] && echo yes || echo no"), "no");
    assert_eq!(
        shell(r#"l=2024-05; [[ $l =~ ^([0-9]{4})-([0-9]{2}) ]]; echo "${BASH_REMATCH[1]}""#),
        "2024"
    );
    // A coordinate still reads on the *left*, which is the side that holds a value.
    assert_eq!(
        shell("cat hosts.txt | [[ {0:0} =~ ^web ]] && echo matched"),
        "matched"
    );
}

/// **`{%n}` is the stage's command, where `{n}` is its output.**
///
/// The ask this answers: "ran <cat> on <one.txt> and got <content>" — a message naming what it did
/// as well as what came back. Both halves of a stage are addressable, and they are addressed the
/// same way.
#[test]
fn a_percent_reads_the_command_rather_than_its_output() {
    assert_eq!(shell("cat hosts.txt | echo {%0}"), "cat hosts.txt");
    assert_eq!(shell("cat hosts.txt | echo {%0:0}"), "cat");
    assert_eq!(shell("cat hosts.txt | echo {%0:1}"), "hosts.txt");
    assert_eq!(shell("cat hosts.txt | echo {%0:-1}"), "hosts.txt");
    assert_eq!(shell("cat hosts.txt | echo {%0:*}"), "cat hosts.txt");
    // The two halves of the same stage, side by side.
    assert_eq!(
        shell("cat hosts.txt | echo cmd={%0:0} out={0:0}"),
        "cmd=cat out=web-01"
    );
    // And a stage further back, exactly as the output axis counts.
    assert_eq!(
        shell("cat hosts.txt | grep db | echo {%0:0} after {%1:0}"),
        "grep after cat"
    );
}

/// A command that was never run reads empty, the way an uncaptured stream does.
#[test]
fn a_command_that_is_not_there_reads_empty() {
    assert_eq!(shell("cat hosts.txt | echo [{%9:0}]"), "[]");
    assert_eq!(shell("echo [{%0:0}]"), "[]");
    // Not a coordinate at all, so brace expansion keeps it.
    assert_eq!(shell("echo {%a}"), "{%a}");
}
/// **Single quotes are text and double quotes expand**, which is the rule the shell already has for
/// every other expansion — `echo "$x"` is the value, `echo '$x'` is the characters.
///
/// The double-quoted half is what makes a coordinate usable in a *message* rather than only as a
/// bare argument, which is most of what anyone wants to write.
#[test]
fn single_quotes_are_text_and_double_quotes_expand() {
    assert_eq!(shell("cat hosts.txt | echo '{0:0}'"), "{0:0}");
    assert_eq!(shell("cat hosts.txt | echo \"{0:0}\""), "web-01");
    assert_eq!(
        shell(r#"cat hosts.txt | echo "ran {%0:0} on {%0:1} and got {0:0}""#),
        "ran cat on hosts.txt and got web-01"
    );
    // Inside quotes the values join and the word stays one word, as `"${a[*]}"` does.
    assert_eq!(
        shell(r#"cat hosts.txt | printf '[%s]\n' "{*:0}""#),
        "[web-01 web-02 db-01]"
    );
    assert_eq!(
        shell(r"cat hosts.txt | printf '[%s]\n' {*:0}"),
        "[web-01]\n[web-02]\n[db-01]"
    );
}

/// A malformed coordinate is left as text — no panic, no crash, and no swallowing of a brace group.
#[test]
fn a_malformed_coordinate_is_left_alone() {
    for text in [
        "{}",
        "{:::}",
        "{0:1:2:3}",
        "{--1}",
        "{-}",
        "{0:0",
        "{ 0:0 }",
        // Far past what an index can hold: refused rather than overflowing.
        "{999999999999999999999}",
    ] {
        assert_eq!(
            shell(&format!("cat hosts.txt | echo [{text}]")),
            format!("[{text}]"),
            "for {text}"
        );
    }
}

/// **A lone command runs in this shell, not in a child.**
///
/// The coordinate path runs stages one at a time in forks, which is right for a pipeline and wrong
/// for a command that is not one: `declare -a b="(y{1,2})"` took this path because the word merely
/// *looked* like it held a coordinate, ran in a child, and left `b` unset when the child exited.
/// Every builtin that changes the shell had the same hole under it.
#[test]
fn a_lone_command_still_changes_this_shell() {
    assert_eq!(
        shell(r#"declare -a b="(y{1,2})"; printf '%s\n' "${b[@]}""#),
        "y1\ny2"
    );
    // The same with a coordinate that really parses, so the gate is genuinely open. It reads empty
    // — there is no upstream — and `X` must still be set here, not in a child that exited.
    assert_eq!(shell(r#"export X="{0:0}y"; echo [$X]"#), "[y]");
    assert_eq!(shell(r#"set -- "{0:0}z"; echo [$1]"#), "[z]");
}
