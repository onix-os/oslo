//! Writing a spec out as YAML oslo will read back.
//!
//! oslo's reader is a deliberate **subset** of YAML — see `crates/oslo-shell/src/spec/yaml.rs` —
//! so this writer stays inside the same subset rather than emitting whatever is legal. Every
//! scalar that is not plainly a bare word is double-quoted, because a description is arbitrary
//! English and a flag key is arbitrary punctuation; value lists are flow, which is how carapace
//! writes them; nothing is ever an anchor, a tag, or a plain scalar that could be read as a
//! boolean.

use super::map::Spec;

/// One spec file, comment header and all.
pub fn document(spec: &Spec, command: &str) -> String {
    let mut lines = vec![format!(
        "# Converted from argc's {command}.sh by tools/argc-to-spec. Do not edit by hand."
    )];
    // The file name is what oslo looks a spec up by, so the name written inside it is the file's
    // and not whatever the script happened to call itself.
    let mut root = clone_named(spec, command);
    root.name = command.to_string();
    emit(&root, "", &mut lines);
    lines.push(String::new());
    lines.join("\n")
}

fn clone_named(spec: &Spec, name: &str) -> Spec {
    Spec {
        name: name.to_string(),
        aliases: spec.aliases.clone(),
        description: spec.description.clone(),
        flags: spec.flags.clone(),
        persistent: spec.persistent.clone(),
        flag_values: spec.flag_values.clone(),
        positional: spec.positional.clone(),
        positional_any: spec.positional_any.clone(),
        commands: spec
            .commands
            .iter()
            .map(|c| clone_named(c, &c.name))
            .collect(),
    }
}

fn emit(spec: &Spec, indent: &str, lines: &mut Vec<String>) {
    lines.push(format!("{indent}name: {}", scalar(&spec.name)));
    if !spec.aliases.is_empty() {
        lines.push(format!("{indent}aliases: {}", flow(&spec.aliases)));
    }
    if !spec.description.is_empty() {
        lines.push(format!(
            "{indent}description: {}",
            scalar(&spec.description)
        ));
    }

    for (key, table) in [
        ("flags", &spec.flags),
        ("persistentflags", &spec.persistent),
    ] {
        if table.is_empty() {
            continue;
        }
        lines.push(format!("{indent}{key}:"));
        for (flag, description, nargs) in table {
            // A flag with only a description is written as one; anything more takes the extended
            // notation, which is a flow mapping and stays on its line.
            let written = match nargs {
                Some(n) => format!("{{description: {}, nargs: {n}}}", scalar(description)),
                None => scalar(description),
            };
            lines.push(format!("{indent}  {}: {written}", scalar(flag)));
        }
    }

    let has_completion = !spec.flag_values.is_empty()
        || !spec.positional.is_empty()
        || !spec.positional_any.is_empty();
    if has_completion {
        lines.push(format!("{indent}completion:"));
        if !spec.flag_values.is_empty() {
            lines.push(format!("{indent}  flag:"));
            for (name, values) in &spec.flag_values {
                lines.push(format!("{indent}    {}: {}", scalar(name), flow(values)));
            }
        }
        if !spec.positional.is_empty() {
            lines.push(format!("{indent}  positional:"));
            for values in &spec.positional {
                lines.push(format!("{indent}    - {}", flow(values)));
            }
        }
        if !spec.positional_any.is_empty() {
            lines.push(format!(
                "{indent}  positionalany: {}",
                flow(&spec.positional_any)
            ));
        }
    }

    if !spec.commands.is_empty() {
        lines.push(format!("{indent}commands:"));
        for child in &spec.commands {
            let mut body = Vec::new();
            emit(child, "", &mut body);
            lines.push(format!("{indent}  - {}", body[0]));
            for line in &body[1..] {
                lines.push(format!("{indent}    {line}"));
            }
        }
    }
}

/// A scalar as this writer emits it: a bare word only when it plainly cannot be read as anything
/// else, and a double-quoted string otherwise.
fn scalar(text: &str) -> String {
    let bare = !text.is_empty()
        && text
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-'))
        && !matches!(
            text.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off"
        );
    if bare {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// A `["a", "b"]` list, always flow: that is how carapace writes a value list.
fn flow(values: &[String]) -> String {
    let inner: Vec<String> = values.iter().map(|v| scalar(v)).collect();
    format!("[{}]", inner.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_word_stays_bare_and_everything_else_is_quoted() {
        assert_eq!(scalar("build"), "build");
        assert_eq!(scalar("v1.2-rc"), "v1.2-rc");
        assert_eq!(scalar("--file="), "\"--file=\"");
        assert_eq!(scalar("has space"), "\"has space\"");
        // A word YAML would otherwise read as a boolean.
        assert_eq!(scalar("yes"), "\"yes\"");
        assert_eq!(scalar(""), "\"\"");
    }

    /// **The tab is the format.** carapace separates a value from its description with one, so an
    /// emitter that wrote a literal tab into a quoted string would be writing a value nobody can
    /// split back apart.
    #[test]
    fn a_tab_survives_as_an_escape() {
        assert_eq!(scalar("main\tthe trunk"), "\"main\\tthe trunk\"");
        assert_eq!(scalar("a\\b"), "\"a\\\\b\"");
        assert_eq!(scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn a_spec_comes_out_as_a_document() {
        let spec = Spec {
            name: "demo".into(),
            aliases: vec!["d".into()],
            description: "a demo".into(),
            flags: vec![("-v, --verbose".into(), "say more".into(), None)],
            persistent: vec![("--config=".into(), "which one".into(), Some(2))],
            flag_values: vec![("config".into(), vec!["a.toml".into()])],
            positional: vec![vec!["one".into(), "two\tthe second".into()]],
            positional_any: vec!["$files".into()],
            commands: vec![Spec {
                name: "sub".into(),
                description: "under it".into(),
                ..Spec::default()
            }],
        };
        let text = document(&spec, "demo");
        assert!(
            text.starts_with("# Converted from argc's demo.sh"),
            "{text}"
        );
        assert!(
            text.contains("\n  \"-v, --verbose\": \"say more\""),
            "{text}"
        );
        assert!(text.contains("nargs: 2}"), "the extended notation: {text}");
        assert!(text.contains("\n    config: [a.toml]"), "{text}");
        assert!(
            text.contains("\n    - [one, \"two\\tthe second\"]"),
            "{text}"
        );
        assert!(text.contains("\n  positionalany: [\"$files\"]"), "{text}");
        assert!(text.contains("\ncommands:\n  - name: sub"), "{text}");
        assert!(text.ends_with('\n'), "a file ends in a newline");
    }
}
