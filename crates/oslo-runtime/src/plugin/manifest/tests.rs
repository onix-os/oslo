use super::*;

/// A plugin directory holding `plugin.lua` and an entry file.
fn plugin(name: &str, manifest: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let root = tempfile::tempdir().expect("temp dir");
    let directory = root.path().join(name);
    std::fs::create_dir_all(&directory).expect("mkdir");
    std::fs::write(directory.join(FILE), manifest).expect("manifest");
    std::fs::write(directory.join("init.lua"), "-- nothing\n").expect("entry");
    (root, directory)
}

#[test]
fn a_manifest_says_what_it_reserves() {
    let (_root, directory) = plugin(
        "notes",
        r#"return {
            name = "notes",
            version = "1.2.3",
            entry = "init.lua",
            builtins = { "note" },
            tools = { "notes" },
        }"#,
    );
    let read = read(&directory).expect("read");
    assert_eq!(read.name, "notes");
    assert_eq!(read.version, "1.2.3");
    assert_eq!(read.entry, "init.lua");
    assert_eq!(read.builtins, ["note"]);
    assert_eq!(read.tools, ["notes"]);
    assert_eq!(
        read.names().collect::<Vec<_>>(),
        vec!["note", "notes"],
        "both kinds of name are reserved together"
    );
}

#[test]
fn the_entry_defaults_and_the_version_does_too() {
    let (_root, directory) = plugin(
        "notes",
        r#"return { name = "notes", builtins = { "note" } }"#,
    );
    let read = read(&directory).expect("read");
    assert_eq!(read.entry, "init.lua");
    assert_eq!(read.version, "0");
}

/// **A manifest is read before anybody has decided to trust the plugin**, so it must not be able to
/// do anything while being read.
#[test]
fn a_manifest_has_no_shell_to_reach() {
    let (_root, directory) = plugin(
        "notes",
        r#"
        if oslo ~= nil then error("the manifest could reach the shell") end
        return { name = "notes", builtins = { "note" } }
        "#,
    );
    read(&directory).expect("a manifest that checks for `oslo` finds nothing and still returns");
}

#[test]
fn a_name_that_is_not_the_directorys_is_refused() {
    let (_root, directory) = plugin(
        "notes",
        r#"return { name = "history", builtins = { "note" } }"#,
    );
    let refused = read(&directory).expect_err("should refuse");
    assert!(refused.contains("sits in a directory"), "{refused}");
}

#[test]
fn an_entry_outside_the_plugins_directory_is_refused() {
    for entry in ["../init.lua", "/etc/init.lua", "sub/init.lua", ".hidden"] {
        let (_root, directory) = plugin(
            "notes",
            &format!(r#"return {{ name = "notes", entry = {entry:?}, builtins = {{ "note" }} }}"#),
        );
        assert!(read(&directory).is_err(), "{entry:?} was accepted");
    }
}

#[test]
fn a_plugin_that_reserves_nothing_is_refused() {
    let (_root, directory) = plugin("notes", r#"return { name = "notes" }"#);
    let refused = read(&directory).expect_err("should refuse");
    assert!(refused.contains("nothing would ever load it"), "{refused}");
}

/// A name the parser would take for itself, or that nobody could type, is not a command name.
#[test]
fn a_name_the_shell_could_not_dispatch_is_refused() {
    for bad in ["if", "done", "two words", "a/b", "with$dollar", ""] {
        let (_root, directory) = plugin(
            "notes",
            &format!(r#"return {{ name = "notes", builtins = {{ {bad:?} }} }}"#),
        );
        assert!(read(&directory).is_err(), "{bad:?} was accepted");
    }
    for good in ["note", "note-2", "note_x", "a.b", "g++"] {
        let (_root, directory) = plugin(
            "notes",
            &format!(r#"return {{ name = "notes", builtins = {{ {good:?} }} }}"#),
        );
        assert!(read(&directory).is_ok(), "{good:?} was refused");
    }
}

#[test]
fn a_manifest_that_is_not_a_table_or_will_not_parse_is_a_message() {
    let (_root, directory) = plugin("notes", "return 3");
    assert!(read(&directory).expect_err("refused").contains("table"));

    let (_root, directory) = plugin("notes", "return {");
    assert!(read(&directory).is_err(), "a syntax error must not panic");
}
