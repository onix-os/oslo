use super::*;

fn literal(text: &str) -> (String, Option<String>) {
    match entry(text).piece {
        Some(Piece::Literal { value, description }) => (value, description),
        other => panic!("not a literal: {other:?}"),
    }
}

#[test]
fn a_value_carries_a_description_and_drops_the_style() {
    assert_eq!(literal("dev"), ("dev".to_string(), None));
    assert_eq!(
        literal("dev\tthe local one"),
        ("dev".to_string(), Some("the local one".to_string()))
    );
    // The third field is an elvish style, which oslo paints from its own theme.
    assert_eq!(
        literal("dev\tthe local one\tblue"),
        ("dev".to_string(), Some("the local one".to_string()))
    );
}

#[test]
fn a_macro_is_a_name_and_a_raw_argument() {
    assert_eq!(
        entry("$files([.go, go.mod])").piece,
        Some(Piece::Macro {
            name: "files".into(),
            arg: "[.go, go.mod]".into()
        })
    );
    assert_eq!(
        entry("$directories").piece,
        Some(Piece::Macro {
            name: "directories".into(),
            arg: String::new()
        })
    );
}

/// **`$(cmd)` is the macro with no name.** That is how carapace spells "run this in a shell", and
/// reading it as a macro called `(cmd)` is how every spec that uses one would stop working.
#[test]
fn the_shell_macro_has_no_name() {
    assert_eq!(
        entry("$(git branch --format '%(refname:short)')").piece,
        Some(Piece::Macro {
            name: String::new(),
            arg: "git branch --format '%(refname:short)'".into()
        })
    );
}

#[test]
fn modifiers_come_after_a_triple_pipe() {
    let e = entry("$files ||| $chdir(/tmp) ||| $tag(sources)");
    assert!(matches!(e.piece, Some(Piece::Macro { .. })));
    assert_eq!(
        e.modifiers,
        vec![
            Modifier::Chdir("/tmp".into()),
            Modifier::Tag("sources".into())
        ]
    );
}

/// A modifier written on its own applies to everything the entries before it produced, so it has
/// no value of its own.
#[test]
fn a_modifier_alone_produces_nothing() {
    let e = entry("$list(,)");
    assert_eq!(e.piece, None);
    assert_eq!(e.modifiers, vec![Modifier::List(",".into())]);
}

/// The modifiers oslo has nothing to do with are still *read*, so a spec using one is not mistaken
/// for a spec declaring a macro that does not exist.
#[test]
fn a_modifier_oslo_ignores_is_still_a_modifier() {
    for text in ["$style(underlined)", "$nospace(/)", "$usage(custom)"] {
        assert_eq!(entry(text).piece, None, "{text}");
        assert_eq!(entry(text).modifiers, vec![Modifier::Ignored], "{text}");
    }
}

#[test]
fn a_bracket_list_is_split_and_a_bare_word_is_a_list_of_one() {
    assert_eq!(bracketed("[.go, go.mod]"), vec![".go", "go.mod"]);
    assert_eq!(bracketed(".go"), vec![".go"]);
    assert!(bracketed("").is_empty());
    assert!(bracketed("[]").is_empty());
}

#[test]
fn the_line_answers_for_its_own_variables() {
    let mut flags = HashMap::new();
    flags.insert("FILE".to_string(), "out.txt".to_string());
    let query = Query {
        args: vec!["build".into()],
        value: "partial".into(),
        flags,
        ..Query::default()
    };
    assert_eq!(query.variable("C_ARG0").as_deref(), Some("build"));
    assert_eq!(query.variable("C_ARG1"), None);
    assert_eq!(query.variable("C_FLAG_FILE").as_deref(), Some("out.txt"));
    assert_eq!(query.variable("C_VALUE").as_deref(), Some("partial"));
}
