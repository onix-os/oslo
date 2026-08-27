use super::*;
use oslo_ui::spec::{Arg, Nargs};

/// The example from the front page of the carapace-spec book, read whole.
const FRONT_PAGE: &str = r#"
name: mycmd
description: my command
flags:
  --optarg?: optarg flag
  -r, --repeatable*: repeatable flag
  -v=: flag with value
persistentflags:
  --help: bool flag
completion:
  flag:
    optarg: ["one", "two\twith description", "three\twith style\tblue"]
    v: ["$files"]
commands:
- name: sub
  description: subcommand
  completion:
    positional:
      - ["$list(,)", "1", "2", "3"]
      - ["$directories"]
"#;

fn front_page() -> CommandSpec {
    spec(FRONT_PAGE).expect("the documented example reads")
}

#[test]
fn the_command_and_its_flags_come_across() {
    let spec = front_page();
    assert_eq!(spec.name, "mycmd");
    assert_eq!(spec.description, "my command");

    let names: Vec<&str> = spec
        .options
        .iter()
        .map(|opt| opt.names[0].as_str())
        .collect();
    assert_eq!(names, vec!["--optarg", "-r", "-v"]);
    assert_eq!(spec.options[0].takes, Arg::Optional);
    assert_eq!(spec.options[0].description, "optarg flag");
    assert!(spec.options[1].repeatable);
    assert_eq!(spec.options[1].names, vec!["-r", "--repeatable"]);
    assert_eq!(spec.options[2].takes, Arg::Required);
}

#[test]
fn a_persistent_flag_lands_where_subcommands_can_see_it() {
    let spec = front_page();
    assert_eq!(spec.persistent.len(), 1);
    assert_eq!(spec.persistent[0].names, vec!["--help"]);
    assert!(spec.options.iter().all(|opt| opt.names != vec!["--help"]));
}

/// **`completion.flag` keys on the longhand**, and the values belong to the flag rather than to a
/// second table the walk would have to carry around.
#[test]
fn a_flags_values_are_attached_to_the_flag() {
    let spec = front_page();
    let optarg = &spec.options[0];
    assert!(
        matches!(&optarg.values, Action::List(list) if list[1] == "two\twith description"),
        "{:?}",
        optarg.values
    );
    let with_value = &spec.options[2];
    assert!(matches!(&with_value.values, Action::List(list) if list == &["$files"]));
    // …and a flag nothing was declared for keeps nothing.
    assert!(spec.options[1].values.is_none());
}

#[test]
fn positions_are_read_in_order() {
    let spec = front_page();
    let sub = &spec.subcommands[0];
    assert_eq!(sub.name, "sub");
    assert_eq!(sub.description, "subcommand");
    assert_eq!(sub.positional.len(), 2);
    assert!(matches!(&sub.positional[0], Action::List(l) if l[0] == "$list(,)" && l.len() == 4));
    assert!(matches!(&sub.positional[1], Action::List(l) if l == &["$directories"]));
    assert!(sub.positional_any.is_none());
}

#[test]
fn the_rest_of_the_command_fields_are_read() {
    let spec = spec(
        "name: c\naliases: [a, al]\nhidden: true\nparsing: non-interspersed\ncompletion:\n  positionalany: [one, two]\n  dashany: [three]\n  dash:\n    - [d1]\n",
    )
    .unwrap();
    assert_eq!(spec.aliases, vec!["a", "al"]);
    assert!(spec.hidden);
    assert_eq!(spec.parsing, Parsing::NonInterspersed);
    assert!(matches!(&spec.positional_any, Action::List(l) if l == &["one", "two"]));
    assert!(matches!(&spec.dash_any, Action::List(l) if l == &["three"]));
    assert_eq!(spec.dash.len(), 1);
}

/// The extended flag notation: `nargs` and `default` against a flow mapping.
#[test]
fn the_extended_flag_notation_is_read() {
    let spec = spec(
        "name: c\nflags:\n  --nargs-two=: {description: consumes two arguments, nargs: 2}\n  --nargs-any=: {description: consumes multiple, nargs: -1}\n  --default-value=: {description: has a default, default: /tmp/out.txt}\n",
    )
    .unwrap();
    assert_eq!(spec.options[0].nargs, Nargs::Exactly(2));
    assert_eq!(spec.options[0].description, "consumes two arguments");
    assert_eq!(spec.options[1].nargs, Nargs::Any);
    assert_eq!(spec.options[2].default.as_deref(), Some("/tmp/out.txt"));
}

/// carapace allows a usage line where a name goes. The first word is the command; the rest is for
/// a person reading the file.
#[test]
fn a_usage_line_is_read_down_to_its_command() {
    let spec = spec("name: usage [-F file | -D dir]... [-f format] profile\n").unwrap();
    assert_eq!(spec.name, "usage");
}

#[test]
fn a_spec_with_no_name_is_refused() {
    assert!(spec("description: nameless\n").is_err());
    assert!(spec("name: [not, a, name]\n").is_err());
}

/// **What is not read is passed over, not choked on.** `run` and `exclusiveflags` are in real spec
/// files, and a reader that failed on them would read almost nothing.
#[test]
fn the_fields_oslo_does_not_read_do_not_stop_it() {
    let spec = spec(
        "name: c\ngroup: grouped\nexclusiveflags:\n  - [add, delete]\nrun: \"[tail, --lines, '1']\"\ndocumentation:\n  command: a paragraph\nflags:\n  --add: add package\n",
    )
    .expect("reads");
    assert_eq!(spec.options.len(), 1);
    assert_eq!(spec.options[0].names, vec!["--add"]);
}
