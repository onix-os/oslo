use super::*;

/// The registry has to be filled, since `columns_at` asks it whether a word is a tool — and it is
/// process-wide, so this takes the turn that `data::tool`'s own test takes before emptying it.
fn with_tools<T>(body: impl FnOnce() -> T) -> T {
    let _turn = super::super::tool::registry_turn();
    super::super::tools::register_all();
    body()
}

fn at_end(line: &str) -> Option<Vec<String>> {
    with_tools(|| columns_at(line, line.len())).map(|found| found.columns)
}

/// The byte offset a chosen candidate is written at.
fn splice(line: &str) -> Option<usize> {
    with_tools(|| columns_at(line, line.len())).map(|found| found.replace_from)
}

/// The headline: what a person actually wants at a structured prompt.
#[test]
fn a_producers_columns_are_offered_to_the_next_stage() {
    let offered = at_end("ls | sort-by ").expect("a column position");
    assert_eq!(offered, super::super::tools::system::LS_COLUMNS);
}

/// A partly typed word is still a column position — the menu filters, this only says what there is.
#[test]
fn a_half_typed_column_is_still_a_column_position() {
    let offered = at_end("ls | sort-by mod").expect("a column position");
    assert!(offered.iter().any(|c| c == "modified"));
}

/// Every verb that names a column offers them, at the operand that names one.
#[test]
fn each_column_naming_verb_offers_them() {
    for line in [
        "ls | cols ",
        "ls | reject ",
        "ls | get ",
        "ls | sort-by ",
        "ls | group-by ",
        "ls | stats ",
        "ls | histogram ",
        "ls | rename ",
        "ls | distinct ",
        "ls | compact ",
        "ls | update ",
    ] {
        let offered = at_end(line).unwrap_or_else(|| panic!("`{line}` is a column position"));
        assert!(offered.contains(&"name".to_string()), "{line}");
    }
}

/// **Not every operand is a column.** The command's own arguments are paths, and an expression is an
/// expression — offering column names there would be worse than offering nothing.
#[test]
fn a_position_that_is_not_a_column_says_so() {
    for line in [
        // `ls` takes a directory.
        "ls ",
        // `first` takes a count.
        "ls | first ",
        // The command word itself.
        "ls | ",
        // `rename`'s second operand is the *new* name, which by definition is not there yet.
        "ls | rename size ",
        // `get` reads one operand and no more.
        "ls | get name ",
    ] {
        assert_eq!(at_end(line), None, "`{line}` must fall through");
    }
}

/// A flag is not a key, so the position after one is still a key position.
#[test]
fn a_sort_flag_is_not_a_column() {
    assert_eq!(at_end("ls | sort-by -"), None, "a flag being typed");
    let offered = at_end("ls | sort-by -r ").expect("still a key position");
    assert!(offered.contains(&"size".to_string()));
}

/// **The algebra follows the line.** A column a verb made is a column the next stage can name, and
/// one it removed is not.
#[test]
fn the_offer_follows_the_pipeline() {
    let offered = at_end("ls | reject size | sort-by ").expect("a column position");
    assert!(offered.contains(&"name".to_string()));
    assert!(!offered.contains(&"size".to_string()), "reject removed it");

    let renamed = at_end("ls | rename size bytes | sort-by ").expect("a column position");
    assert!(renamed.contains(&"bytes".to_string()));
    assert!(!renamed.contains(&"size".to_string()));

    let grouped = at_end("ps | group-by name | cols ").expect("a column position");
    assert_eq!(grouped, vec!["name", "count", "rows"]);
}

/// **A column position with nothing knowable answers nothing, not "fall through".** Offering
/// filenames where a column belongs is the wrong nothing.
#[test]
fn an_unknowable_stream_offers_nothing_rather_than_falling_through() {
    let offered = at_end("cat x.json | from json | cols ").expect("still a column position");
    assert!(offered.is_empty());
}

/// `parse` says what it produces, so the stage after it can be completed from the pattern alone.
#[test]
fn parse_offers_the_columns_its_pattern_names() {
    let offered = at_end("cat /etc/passwd | parse '{user}:{x}:{uid}' | cols ").expect("a position");
    assert_eq!(offered, vec!["user", "x", "uid"]);
}

/// An external upstream ends what is known, and the offer is empty rather than wrong.
#[test]
fn an_external_upstream_knows_nothing() {
    let offered = at_end("ls | grep x | cols ").expect("a column position");
    assert!(offered.is_empty());
}

/// A pipe inside quotes is not a pipe, and `||` is not one either — both would cut a stage in the
/// wrong place and offer the columns of something that is not upstream.
#[test]
fn only_a_real_pipe_splits_a_stage() {
    // The `|` is inside the filter, so `cols` is still the second stage of `ls | …`.
    let offered = at_end("ls | where 'name:match(\"a|b\")' | cols ").expect("a column position");
    assert!(offered.contains(&"name".to_string()));

    // After `||` nothing is upstream.
    let after_or = at_end("ls || ls | cols ").expect("a column position");
    assert!(
        after_or.contains(&"name".to_string()),
        "the right side's own ls"
    );

    // And after a `;` the earlier pipeline is gone.
    let after_semicolon = at_end("ps ; ls | cols ").expect("a column position");
    assert!(
        after_semicolon.contains(&"modified".to_string()),
        "ls, not ps"
    );
    assert!(!after_semicolon.contains(&"cmdline".to_string()));
}

/// The words splitter keeps a quoted operand whole.
#[test]
fn a_quoted_operand_is_one_word() {
    assert_eq!(words_of("cols 'a b' c"), ["cols", "a b", "c"]);
    assert_eq!(words_of("  spaced   out  "), ["spaced", "out"]);
    assert_eq!(words_of(""), Vec::<String>::new());
    // An empty quoted word is a word.
    assert_eq!(
        words_of("reduce --from '' 'acc'"),
        ["reduce", "--from", "", "acc"]
    );
}

/// Nothing typed at all is not a column position, and neither is a line with no pipe.
#[test]
fn a_line_with_no_upstream_is_quiet() {
    assert_eq!(at_end(""), None);
    assert_eq!(at_end("ls"), None);
    // A verb with no producer before it: still a column position, but nothing is known.
    let bare = at_end("cols ").expect("a column position");
    assert!(bare.is_empty());
}

/// **The filter is where a column name is most often typed**, and it was the one place with no help
/// — the word under the cursor is a quoted expression rather than an operand, so the operand rule
/// never reached it.
#[test]
fn a_column_inside_a_filter_is_offered() {
    for line in ["ls | where '", "ls | map '", "ls | each '"] {
        let offered = at_end(line).unwrap_or_else(|| panic!("`{line}` names columns"));
        assert!(offered.contains(&"size".to_string()), "{line}: {offered:?}");
    }
    // And the three that compute, whose expression is the *second* operand.
    let computed = at_end("ls | insert kb '").expect("an expression position");
    assert!(computed.contains(&"size".to_string()), "{computed:?}");
}

/// **Only the identifier is replaced, not the expression around it.** Splicing at the word's start
/// would overwrite `size > 1 and nam` with one column name.
#[test]
fn only_the_identifier_under_the_cursor_is_replaced() {
    let line = "ls | where 'size > 1 and nam";
    assert!(
        at_end(line)
            .expect("a column position")
            .contains(&"name".to_string())
    );
    let at = splice(line).expect("a splice point");
    assert_eq!(&line[at..], "nam", "the identifier, and nothing before it");

    // At the very start of an expression there is nothing typed yet.
    let empty = "ls | where '";
    assert_eq!(splice(empty), Some(empty.len()));
}

/// **A name after a `.` or a `:` is not a column.** `row.na` is a field of the row and `name:up` is
/// a method on a string; offering the stream's columns there would splice one into the middle of an
/// access that means something else.
#[test]
fn a_field_or_a_method_is_not_a_column() {
    assert_eq!(at_end("ls | where 'row.na"), None);
    assert_eq!(at_end("ls | where 'name:ma"), None);
    // A number is not a half-typed name either.
    assert_eq!(at_end("ls | where 'size > 10"), None);
}

/// A bare operand is still replaced whole, which is what keeps `sort-by mod` working.
#[test]
fn a_bare_operand_still_replaces_the_whole_word() {
    let line = "ls | sort-by mod";
    let at = splice(line).expect("a splice point");
    assert_eq!(&line[at..], "mod");
}

/// `reduce --from '' 'acc + siz` — the flag pushes the expression along, and the columns are still
/// bound inside it.
#[test]
fn reduce_finds_its_expression_after_the_flag() {
    let offered = at_end("ls | reduce --from 0 'acc + siz").expect("an expression position");
    assert!(offered.contains(&"size".to_string()), "{offered:?}");
}
