//! VS Code's rich OSC 633 shell-integration encoder.

const OSC: &str = "\x1b]633;";
const ST: &str = "\x1b\\";

pub fn prompt_start() -> String {
    sequence("A")
}

pub fn prompt_end() -> String {
    sequence("B")
}

pub fn command_start() -> String {
    sequence("C")
}

pub fn command_end(status: Option<i32>) -> String {
    match status {
        Some(status) => sequence(&format!("D;{status}")),
        None => sequence("D"),
    }
}

/// Publish the exact command after escaping VS Code's field separators.
pub fn command_line(command: &str, nonce: Option<&str>) -> String {
    let mut body = format!("E;{}", escape(command));
    if let Some(nonce) = nonce {
        body.push(';');
        body.push_str(&escape(nonce));
    }
    sequence(&body)
}

pub fn property(name: &str, value: &str) -> Option<String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    Some(sequence(&format!("P;{name}={}", escape(value))))
}

fn sequence(body: &str) -> String {
    format!("{OSC}{body}{ST}")
}

fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            ';' => escaped.push_str("\\x3b"),
            character if character.is_ascii() && character < ' ' => {
                escaped.push_str(&format!("\\x{:02x}", character as u32));
            }
            '\u{7f}' => escaped.push_str("\\x7f"),
            character => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_has_the_required_ordering_primitives() {
        assert_eq!(prompt_start(), "\x1b]633;A\x1b\\");
        assert_eq!(prompt_end(), "\x1b]633;B\x1b\\");
        assert_eq!(command_start(), "\x1b]633;C\x1b\\");
        assert_eq!(command_end(Some(7)), "\x1b]633;D;7\x1b\\");
        assert_eq!(command_end(None), "\x1b]633;D\x1b\\");
    }

    #[test]
    fn command_text_cannot_split_the_osc_field() {
        assert_eq!(
            command_line("printf 'a;b'\nnext\\path", None),
            "\x1b]633;E;printf 'a\\x3bb'\\x0anext\\\\path\x1b\\"
        );
    }

    #[test]
    fn unicode_is_kept_and_a_nonce_uses_the_same_escaping() {
        assert_eq!(
            command_line("echo café", Some("a;b")),
            "\x1b]633;E;echo café;a\\x3bb\x1b\\"
        );
    }

    #[test]
    fn property_names_are_narrow_and_values_are_escaped() {
        assert_eq!(
            property("Cwd", "/tmp/a;b"),
            Some("\x1b]633;P;Cwd=/tmp/a\\x3bb\x1b\\".to_string())
        );
        assert_eq!(property("bad=name", "x"), None);
        assert_eq!(property("", "x"), None);
    }
}
