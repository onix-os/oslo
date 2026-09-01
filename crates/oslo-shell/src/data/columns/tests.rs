use super::*;
use crate::data::{Record, Val};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

fn of(name: &str, words: &[&str], input: &Columns) -> Columns {
    let mut all = vec![name.to_string()];
    all.extend(argv(words));
    through(name, &all, input)
}

fn known(names: &[&str]) -> Columns {
    Columns::known(names)
}

/// **The declaration must not be able to lie.** A producer that says one thing and builds another
/// turns the planner's refusal into a wrong answer, which is the failure this whole idea would
/// otherwise introduce — so the constants are checked against a real run.
#[test]
fn a_producer_answers_the_columns_it_declares() {
    // `ls` on a directory that certainly exists and certainly has entries.
    let rows = crate::data::tools::system::ls(&[".".to_string()]).expect("ls .");
    let row = rows.first().expect("at least one entry in the source tree");
    assert_eq!(
        row.columns(),
        crate::data::tools::system::LS_COLUMNS,
        "ls declares one set and builds another"
    );

    let processes = crate::data::tools::system::ps();
    let process = processes.first().expect("at least one process");
    assert_eq!(
        process.columns(),
        crate::data::tools::system::PS_COLUMNS,
        "ps declares one set and builds another"
    );

    // `df` shells out, so it is allowed to be unavailable; when it answers, it must match.
    if let Ok(filesystems) = crate::data::tools::df::rows()
        && let Some(filesystem) = filesystems.first()
    {
        assert_eq!(
            filesystem.columns(),
            crate::data::tools::df::COLUMNS,
            "df declares one set and builds another"
        );
    }

    // `history` is not run here: it reads a store a test process has not opened, so it answers no
    // rows and there is nothing to compare. The shape it *would* build is pinned against `COLUMNS`
    // beside the code that builds it, in `tools::past`.
}

/// The producers answer exactly, which is what makes the head of a real pipeline knowable.
#[test]
fn the_producers_are_known() {
    assert_eq!(
        of("ls", &[], &Columns::Unknown),
        known(crate::data::tools::system::LS_COLUMNS)
    );
    assert_eq!(
        of("df", &[], &Columns::Unknown),
        known(crate::data::tools::df::COLUMNS)
    );
    assert_eq!(
        of("history", &[], &Columns::Unknown),
        known(crate::data::tools::past::COLUMNS)
    );
}

/// **`parse` says what it produces**, which is the find that makes this worth doing: the columns are
/// in a literal operand, so a pipeline reading `/etc/passwd` is knowable before a byte arrives.
#[test]
fn parse_reads_its_columns_out_of_the_pattern() {
    assert_eq!(
        of("parse", &["{user}:{x}:{uid}:{rest}"], &Columns::Unknown),
        known(&["user", "x", "uid", "rest"])
    );
    assert_eq!(
        of(
            "parse",
            &["--regex", r"(?<ip>\S+) (?<when>\S+)"],
            &Columns::Unknown
        ),
        known(&["ip", "when"])
    );
    // A pattern with no holes, and a regex with no named groups, are refused when they run — this
    // says nothing rather than guessing.
    assert_eq!(
        of("parse", &["no holes"], &Columns::Unknown),
        Columns::Unknown
    );
    assert_eq!(
        of("parse", &["--regex", r"(\w+)"], &Columns::Unknown),
        Columns::Unknown
    );
    assert_eq!(
        of("parse", &["--regex", "(?<a>["], &Columns::Unknown),
        Columns::Unknown
    );
}

/// The bridges that learn their columns from the data say so.
#[test]
fn the_data_driven_bridges_are_unknown() {
    for (name, words) in [("from", vec!["json"]), ("detect-columns", vec![])] {
        assert_eq!(
            of(name, &words, &Columns::Unknown),
            Columns::Unknown,
            "{name}"
        );
    }
}

/// A verb that only chooses rows keeps the columns it was given.
#[test]
fn the_row_verbs_pass_the_shape_through() {
    let input = known(&["a", "b"]);
    for name in [
        "where", "sort-by", "reverse", "first", "final", "skip", "every", "distinct", "compact",
    ] {
        assert_eq!(of(name, &[], &input), input, "{name}");
    }
}

/// The verbs that name their own output.
#[test]
fn the_reshaping_verbs_name_what_they_make() {
    let input = known(&["name", "size", "mode"]);
    assert_eq!(
        of("cols", &["size", "name"], &input),
        known(&["size", "name"])
    );
    assert_eq!(of("get", &["size"], &input), known(&["size"]));
    assert_eq!(of("reject", &["mode"], &input), known(&["name", "size"]));
    assert_eq!(
        of("rename", &["size", "bytes"], &input),
        known(&["name", "bytes", "mode"]),
        "renamed in place, because a record's order is not incidental"
    );
    assert_eq!(
        of("insert", &["kb"], &input),
        known(&["name", "size", "mode", "kb"])
    );
    assert_eq!(
        of("update", &["size"], &input),
        known(&["name", "size", "mode"]),
        "updating a column that is there adds nothing"
    );
    assert_eq!(
        of("enumerate", &[], &input),
        known(&["index", "name", "size", "mode"]),
        "the index leads"
    );
    assert_eq!(of("length", &[], &input), known(&["length"]));
    assert_eq!(of("reduce", &["acc + size"], &input), known(&["reduced"]));
}

/// The summarising verbs, including the one that notices what it was handed.
#[test]
fn the_summaries_answer_their_own_shapes() {
    let input = known(&["user", "size"]);
    assert_eq!(
        of("group-by", &["user"], &input),
        known(&["user", "count", "rows"])
    );
    assert_eq!(
        of("stats", &["size"], &input),
        known(&["field", "count", "min", "max", "sum", "mean"])
    );
    assert_eq!(
        of("describe", &[], &input),
        known(&["column", "type", "filled", "rows"])
    );
    assert_eq!(
        of("histogram", &["user"], &input),
        known(&["user", "count", "bar"])
    );

    // `count` on a plain stream is one number; after `group-by` it keeps the group's columns and
    // drops the rows it was carrying — the same rule the verb itself follows.
    assert_eq!(of("count", &[], &input), known(&["count"]));
    let grouped = known(&["user", "count", "rows"]);
    assert_eq!(of("count", &[], &grouped), known(&["user", "count"]));
}

/// Where knowledge genuinely ends, and it must end honestly.
#[test]
fn the_opaque_verbs_are_unknown() {
    let input = known(&["a"]);
    for name in [
        "map", "flatten", "headers", "lookup", "append", "merge", "each", "to",
    ] {
        assert_eq!(of(name, &["x"], &input), Columns::Unknown, "{name}");
    }
}

/// An unknown input stays unknown through a verb that only reshapes what it was given.
#[test]
fn unknown_is_infectious() {
    for name in ["reject", "rename", "insert", "enumerate", "where"] {
        assert_eq!(
            of(name, &["a", "b"], &Columns::Unknown),
            Columns::Unknown,
            "{name}"
        );
    }
}

/// **Nothing may be refused on an `Unknown`.** The rule the whole design rests on.
#[test]
fn unknown_accepts_everything() {
    assert!(Columns::Unknown.accepts("anything at all"));
    assert!(Columns::Unknown.accepts("a.b.c"));
}

/// A known set accepts what it has and refuses what it does not.
#[test]
fn a_known_set_knows_what_it_has() {
    let columns = known(&["name", "metadata", "a.b"]);
    assert!(columns.accepts("name"));
    assert!(!columns.accepts("nmae"));
}

/// **Generous about paths on purpose.** Whether `metadata` holds a record with a `name` in it is a
/// question about data, so only the first step can be judged — and an exact column called `a.b`
/// wins outright, exactly as `Path` resolves it.
#[test]
fn a_path_is_judged_by_its_first_step_only() {
    let columns = known(&["name", "metadata", "a.b"]);
    assert!(columns.accepts("metadata.name"), "the first step is there");
    assert!(columns.accepts("metadata.deeply.nested.thing"));
    assert!(!columns.accepts("nope.name"), "the first step is not");
    assert!(columns.accepts("a.b"), "a column really called a.b");
    // An optional first step said the absence was expected.
    assert!(columns.accepts("nope?.name"));
}

/// A `Record` built by a verb and the algebra's answer for that verb must agree — a spot check that
/// the table is describing the code rather than an intention.
#[test]
fn the_algebra_agrees_with_what_a_verb_builds() {
    let rows = vec![Record::from_pairs([
        ("user", Val::Str("root".into())),
        ("size", Val::Size(10)),
    ])];
    let input = known(&["user", "size"]);

    let grouped = crate::data::tools::summarise::group_by(&rows, "user");
    assert_eq!(
        Columns::Known(grouped[0].columns().to_vec()),
        of("group-by", &["user"], &input)
    );

    let described = crate::data::tools::summarise::describe(&rows);
    assert_eq!(
        Columns::Known(described[0].columns().to_vec()),
        of("describe", &[], &input)
    );

    let counted = crate::data::tools::summarise::stats(&rows, "size");
    assert_eq!(
        Columns::Known(counted[0].columns().to_vec()),
        of("stats", &["size"], &input)
    );
}
