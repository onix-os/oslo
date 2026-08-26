use super::*;

fn parsed(source: &str) -> Node {
    parse(source).expect("parses")
}

#[test]
fn a_mapping_keeps_the_order_it_was_written_in() {
    let node = parsed("name: mycmd\ndescription: my command\n");
    assert_eq!(
        node.pairs().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        vec!["name", "description"]
    );
    assert_eq!(node.get("name").and_then(Node::text), Some("mycmd"));
}

#[test]
fn nesting_is_by_indentation() {
    let node = parsed("completion:\n  flag:\n    v: [\"$files\"]\n");
    let flag = node.get("completion").and_then(|c| c.get("flag")).unwrap();
    assert_eq!(
        flag.get("v").map(Node::items),
        Some(vec![&Node::Scalar("$files".into())])
    );
}

/// The shape every generated spec is full of: a sequence of mappings whose first key shares the
/// dash's line.
#[test]
fn a_sequence_of_mappings_reads_the_dashed_line_as_a_key() {
    let node = parsed("commands:\n  - name: sub\n    description: subcommand\n  - name: other\n");
    let commands = node.get("commands").unwrap().items();
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0].get("name").and_then(Node::text), Some("sub"));
    assert_eq!(
        commands[0].get("description").and_then(Node::text),
        Some("subcommand")
    );
    assert_eq!(commands[1].get("name").and_then(Node::text), Some("other"));
}

#[test]
fn a_sequence_of_sequences_nests() {
    let node = parsed("positional:\n  - [\"$list(,)\", \"1\", \"2\"]\n  - [\"$directories\"]\n");
    let positional = node.get("positional").unwrap().items();
    assert_eq!(positional.len(), 2);
    assert_eq!(positional[0].items().len(), 3);
    assert_eq!(positional[1].items()[0].text(), Some("$directories"));
}

/// **`\t` is what separates a value from its description**, so a double-quoted escape is not a
/// nicety here — it is the whole format.
#[test]
fn double_quoted_escapes_are_resolved() {
    let node = parsed("v: [\"one\", \"two\\twith description\", \"three\\tstyled\\tblue\"]\n");
    let items = node.get("v").unwrap().items();
    assert_eq!(items[1].text(), Some("two\twith description"));
    assert_eq!(items[2].text(), Some("three\tstyled\tblue"));
}

#[test]
fn single_quotes_are_literal() {
    let node = parsed(r"a: 'a \t b'");
    assert_eq!(node.get("a").and_then(Node::text), Some(r"a \t b"));
}

/// A flag name is a key, and flag names are full of punctuation: `-v=`, `--optarg?`, `-o, --opt*`.
#[test]
fn a_key_may_be_a_flag_declaration() {
    let node = parsed(
        "flags:\n  --optarg?: optarg flag\n  -r, --repeatable*: repeatable\n  -v=: with value\n",
    );
    let flags = node.get("flags").unwrap();
    assert_eq!(
        flags.pairs().iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        vec!["--optarg?", "-r, --repeatable*", "-v="]
    );
    assert_eq!(flags.get("-v=").and_then(Node::text), Some("with value"));
}

#[test]
fn a_flow_mapping_is_a_mapping() {
    let node = parsed("--nargs-two=: {description: consumes two, nargs: 2}\n");
    let entry = node.get("--nargs-two=").unwrap();
    assert_eq!(
        entry.get("description").and_then(Node::text),
        Some("consumes two")
    );
    assert_eq!(entry.get("nargs").and_then(Node::text), Some("2"));
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let node =
        parsed("# a leading comment\n\nname: c  # and a trailing one\n\n# another\nhidden: true\n");
    assert_eq!(node.get("name").and_then(Node::text), Some("c"));
    assert!(node.get("hidden").unwrap().truthy());
}

/// A `#` that is part of a value is not a comment.
#[test]
fn a_hash_inside_a_value_stays() {
    assert_eq!(
        parsed("run: \"echo #1\"").get("run").and_then(Node::text),
        Some("echo #1")
    );
}

#[test]
fn a_block_scalar_keeps_its_lines() {
    let node = parsed("run: |\n  #!/usr/bin/env bash\n  echo one\n  echo two\nname: after\n");
    assert_eq!(
        node.get("run").and_then(Node::text),
        Some("#!/usr/bin/env bash\necho one\necho two\n")
    );
    assert_eq!(node.get("name").and_then(Node::text), Some("after"));
}

/// **What it does not read, it says so about.** A partial parser that guessed at an anchor would
/// silently complete the wrong word; a message names the line instead.
#[test]
fn the_constructs_outside_the_subset_are_refused() {
    let problem = parse("base: &anchor\n  name: x\n").unwrap_err();
    assert!(problem.contains("anchors"), "{problem}");
    let problem = parse("name: one\n---\nname: two\n").unwrap_err();
    assert!(problem.contains("document"), "{problem}");
}

#[test]
fn a_leading_document_marker_is_allowed() {
    assert_eq!(
        parsed("---\nname: c\n").get("name").and_then(Node::text),
        Some("c")
    );
}
