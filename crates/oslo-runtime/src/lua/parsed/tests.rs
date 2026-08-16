use super::*;

/// The Lua list, as `(argv, link)` per command, which is what every assertion here is about.
fn parsed(text: &str) -> Vec<(Vec<String>, String)> {
    let Some(Value::Table(list)) = commands_of(text) else {
        panic!("{text:?} did not parse");
    };
    let list = list.borrow();
    (1..=list.length())
        .map(|at| {
            let Value::Table(command) = list.get(&Value::int(at)) else {
                panic!("command {at} is not a table");
            };
            let command = command.borrow();
            let Value::Str(link) = command.get_str("link") else {
                panic!("command {at} has no link");
            };
            let argv = match command.get_str("argv") {
                Value::Table(argv) => {
                    let argv = argv.borrow();
                    (1..=argv.length())
                        .map(|w| match argv.get(&Value::int(w)) {
                            Value::Str(s) => s.to_string(),
                            other => panic!("word {w} is {other:?}"),
                        })
                        .collect()
                }
                _ => Vec::new(),
            };
            (argv, link.to_string())
        })
        .collect()
}

/// One command is one entry, and its words are its words.
#[test]
fn a_single_command_is_its_argv() {
    assert_eq!(
        parsed("git commit --all"),
        vec![(
            vec!["git".into(), "commit".into(), "--all".into()],
            "first".into()
        )]
    );
}

/// **The count is the point.** `#c.commands` answers "how many commands does this line run", which
/// is the question a counter is built on.
#[test]
fn every_operator_is_counted_and_named() {
    let line = parsed("a | b && c || d ; e & f");
    let links: Vec<&str> = line.iter().map(|(_, link)| link.as_str()).collect();
    assert_eq!(links, vec!["first", "|", "&&", "||", ";", "&"]);
    assert_eq!(line.len(), 6, "six commands on one line");

    let names: Vec<&str> = line.iter().map(|(argv, _)| argv[0].as_str()).collect();
    assert_eq!(names, vec!["a", "b", "c", "d", "e", "f"]);
}

/// A pipeline's members are `|`, however long it is — the reason a flat list works at all.
#[test]
fn a_long_pipeline_is_flat() {
    let line = parsed("cat x | grep y | sort | uniq -c");
    assert_eq!(line.len(), 4);
    assert_eq!(
        line.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(),
        vec!["first", "|", "|", "|"]
    );
    assert_eq!(line[3].0, vec!["uniq".to_string(), "-c".to_string()]);
}

/// Quoting is resolved, which is the whole reason this exists rather than `gmatch("%S+")`.
#[test]
fn quoting_is_resolved_into_single_words() {
    assert_eq!(
        parsed("cat 'a b'")[0].0,
        vec!["cat".to_string(), "a b".into()]
    );
    assert_eq!(
        parsed(r#"cat "a b""#)[0].0,
        vec!["cat".to_string(), "a b".into()]
    );
    assert_eq!(
        parsed(r"cat a\ b")[0].0,
        vec!["cat".to_string(), "a b".into()]
    );
    // Three words, not two: the quotes are what joined the pair above.
    assert_eq!(parsed("cat a b")[0].0.len(), 3);
}

/// Expansion is *not* resolved, and says so by keeping what was written.
#[test]
fn an_unexpanded_word_is_returned_as_written() {
    assert_eq!(parsed("cat $HOME")[0].0[1], "$HOME");
    assert_eq!(parsed("cat ~/x")[0].0[1], "~/x");
    assert_eq!(parsed("echo $(date)")[0].0[1], "$(date)");
}

/// A compound command still counts, so the total matches what the line runs.
#[test]
fn a_compound_command_is_named_rather_than_dropped() {
    let Some(Value::Table(list)) = commands_of("for i in 1 2; do echo $i; done") else {
        panic!("did not parse");
    };
    assert!(list.borrow().length() >= 1);
    let Value::Table(first) = list.borrow().get(&Value::int(1)) else {
        panic!("no first command");
    };
    let Value::Str(kind) = first.borrow().get_str("kind") else {
        panic!("no kind");
    };
    assert_eq!(kind.to_string(), "compound");
}

/// A line that does not parse is absent rather than empty, so a handler can tell the difference.
#[test]
fn an_unparseable_line_has_no_commands() {
    assert!(commands_of("for i in").is_none());
    assert!(commands_of("echo 'unterminated").is_none());
}

/// An empty line parses to nothing, which is not the same as failing to parse.
#[test]
fn an_empty_line_is_no_commands_rather_than_a_failure() {
    let Some(Value::Table(list)) = commands_of("") else {
        panic!("an empty line parses");
    };
    assert_eq!(list.borrow().length(), 0);
}
