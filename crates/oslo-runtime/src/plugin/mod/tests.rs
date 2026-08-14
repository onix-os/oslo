//! Which line loads which plugin. Loading itself needs a live interpreter and a real home, and is
//! covered end to end by `tests/plugin_tests.rs`.

use super::*;

fn installed(name: &str, builtins: &[&str]) -> index::Installed {
    index::Installed {
        name: name.to_string(),
        entry: "init.lua".to_string(),
        builtins: builtins.iter().map(|n| (*n).to_string()).collect(),
        tools: Vec::new(),
        hash: "x".to_string(),
        requires: None,
        load_on: None,
        secrets: Vec::new(),
    }
}

/// What `ensure_loaded` would pick out of a line, without loading anything.
fn wanted(line: &str, from: &[index::Installed]) -> Vec<String> {
    let words: std::collections::HashSet<&str> = line.split_whitespace().collect();
    from.iter()
        .filter(|entry| entry.names().any(|name| words.contains(name.as_str())))
        .map(|entry| entry.name.clone())
        .collect()
}

/// **Every word, not only the first.** A plugin command in a pipeline or after `&&` is still that
/// plugin's command, and none of those is at the front of the line.
#[test]
fn a_line_naming_a_plugins_command_anywhere_wants_it() {
    let all = [
        installed("notes", &["note"]),
        installed("other", &["thing"]),
    ];
    assert_eq!(wanted("note hello", &all), ["notes"]);
    assert_eq!(wanted("note x | wc -l", &all), ["notes"]);
    assert_eq!(wanted("true && note x", &all), ["notes"]);
    assert_eq!(wanted("echo hi", &all), Vec::<String>::new());
    // Both, when a line names both.
    assert_eq!(wanted("note x; thing y", &all), ["notes", "other"]);
}

/// A name that merely *contains* a command name is not that command.
#[test]
fn a_longer_word_is_not_the_command() {
    let all = [installed("notes", &["note"])];
    assert_eq!(wanted("notepad x", &all), Vec::<String>::new());
    assert_eq!(wanted("./note", &all), Vec::<String>::new());
}

#[test]
fn a_tool_name_counts_as_much_as_a_builtin() {
    let mut with_tool = installed("notes", &["note"]);
    with_tool.tools = vec!["notes-list".to_string()];
    assert_eq!(wanted("notes-list | first 3", &[with_tool]), ["notes"]);
}
