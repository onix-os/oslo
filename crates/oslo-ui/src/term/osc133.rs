//! Generic OSC 133 encoding.

use super::semantic::PromptKind;

const OSC: &str = "\x1b]133;";
const ST: &str = "\x1b\\";

pub fn prompt_start(aid: &str, kind: PromptKind, semantic_clicks: bool) -> String {
    match kind {
        PromptKind::Primary if semantic_clicks => {
            sequence(&format!("A;cl=line;click_events=2;aid={}", clean_aid(aid)))
        }
        PromptKind::Primary => sequence(&format!("A;aid={}", clean_aid(aid))),
        PromptKind::Continuation => sequence(&format!("A;k=s;aid={}", clean_aid(aid))),
        PromptKind::Right => String::new(),
    }
}

pub fn input_start(aid: &str) -> String {
    sequence(&format!("B;aid={}", clean_aid(aid)))
}

pub fn command_start(aid: &str, command: Option<&str>) -> String {
    match command {
        Some(command) => sequence(&format!(
            "C;cmdline_url={};aid={}",
            percent_encode(command),
            clean_aid(aid)
        )),
        None => sequence(&format!("C;aid={}", clean_aid(aid))),
    }
}

pub fn command_end(aid: &str, status: Option<i32>) -> String {
    match status {
        Some(status) => sequence(&format!("D;{status};aid={}", clean_aid(aid))),
        None => sequence(&format!("D;aid={}", clean_aid(aid))),
    }
}

pub fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn clean_aid(aid: &str) -> String {
    aid.chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        .collect()
}

fn sequence(body: &str) -> String {
    format!("{OSC}{body}{ST}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_fields_are_encoded_and_terminated() {
        assert_eq!(
            prompt_start("12-start", PromptKind::Primary, false),
            "\x1b]133;A;aid=12-start\x1b\\"
        );
        assert_eq!(input_start("12-start"), "\x1b]133;B;aid=12-start\x1b\\");
        assert_eq!(
            command_start("12-start", Some("echo café; 10%\n")),
            "\x1b]133;C;cmdline_url=echo%20caf%C3%A9%3B%2010%25%0A;aid=12-start\x1b\\"
        );
        assert_eq!(
            command_end("12-start", Some(7)),
            "\x1b]133;D;7;aid=12-start\x1b\\"
        );
    }

    #[test]
    fn semantic_clicks_are_primary_prompt_properties() {
        assert_eq!(
            prompt_start("12-start", PromptKind::Primary, true),
            "\x1b]133;A;cl=line;click_events=2;aid=12-start\x1b\\"
        );
        assert!(!prompt_start("12-start", PromptKind::Continuation, true).contains("click_events"));
        assert_eq!(prompt_start("12-start", PromptKind::Right, true), "");
    }

    #[test]
    fn fields_cannot_inject_another_sequence() {
        let encoded = command_start("bad;\x1b]0;x", Some("echo\x07;next"));
        assert_eq!(encoded.matches("\x1b]").count(), 1);
        assert!(encoded.contains("aid=bad0x"), "{encoded:?}");
        assert!(encoded.contains("echo%07%3Bnext"), "{encoded:?}");
    }
}
