//! Gated encoders for optional terminal metadata.

use super::capability::Capabilities;
use oslo_base::base64::encode as base64;

const MAX_VALUE: usize = 4096;

/// Encode one OSC 1337 user variable for a verified consumer.
pub fn user_variable(capabilities: &Capabilities, name: &str, value: &str) -> Option<String> {
    if !capabilities.osc1337_user_vars || !valid_identifier(name) || value.len() > MAX_VALUE {
        return None;
    }
    Some(format!(
        "\x1b]1337;SetUserVar={name}={}\x1b\\",
        base64(value.as_bytes())
    ))
}

/// Clear one previously published user variable.
pub fn clear_user_variable(capabilities: &Capabilities, name: &str) -> Option<String> {
    user_variable(capabilities, name, "")
}

/// State changes Oslo can report without inventing a percentage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Indeterminate,
    Complete,
    Clear,
}

/// Encode foreground progress for a verified consumer.
pub fn progress(capabilities: &Capabilities, progress: Progress) -> Option<String> {
    if !capabilities.osc9_progress {
        return None;
    }
    let fields = match progress {
        Progress::Indeterminate => "3",
        Progress::Complete => "1;100",
        Progress::Clear => "0",
    };
    Some(format!("\x1b]9;4;{fields}\x1b\\"))
}

/// Encode exactly one rich notification or its OSC 777 fallback.
pub fn notification(
    capabilities: &Capabilities,
    id: &str,
    title: &str,
    body: &str,
) -> Option<String> {
    if title.len() > MAX_VALUE || body.len() > MAX_VALUE {
        return None;
    }
    if capabilities.osc99_notifications {
        if !valid_identifier(id) {
            return None;
        }
        let title = base64(title.as_bytes());
        let body = base64(body.as_bytes());
        let occasion = if capabilities.osc99_unfocused {
            ":o=unfocused"
        } else {
            ""
        };
        return Some(format!(
            "\x1b]99;i={id}:d=0:e=1:p=title{occasion};{title}\x1b\\\x1b]99;i={id}:d=1:e=1:p=body{occasion};{body}\x1b\\"
        ));
    }

    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| if c == ';' { ',' } else { c })
            .filter(|c| !c.is_control())
            .collect()
    };
    Some(format!(
        "\x1b]777;notify;{};{}\x1b\\",
        clean(title),
        clean(body)
    ))
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::capability::Verified;

    #[test]
    fn user_variables_are_gated_validated_and_encoded() {
        let off = Capabilities::portable();
        assert_eq!(user_variable(&off, "oslo_mode", "lua"), None);

        let mut on = off;
        on.osc1337_user_vars = true;
        assert_eq!(
            user_variable(&on, "oslo_mode", "lua"),
            Some("\x1b]1337;SetUserVar=oslo_mode=bHVh\x1b\\".to_string())
        );
        assert_eq!(user_variable(&on, "bad;name", "lua"), None);
        assert_eq!(user_variable(&on, "", "lua"), None);
        assert_eq!(
            clear_user_variable(&on, "oslo_mode").as_deref(),
            Some("\x1b]1337;SetUserVar=oslo_mode=\x1b\\")
        );
    }

    #[test]
    fn progress_never_fakes_an_unknown_percentage() {
        let off = Capabilities::portable();
        assert_eq!(progress(&off, Progress::Indeterminate), None);
        let on = off.with_verified(Verified {
            osc9_progress: true,
            ..Verified::default()
        });
        assert_eq!(
            progress(&on, Progress::Indeterminate).as_deref(),
            Some("\x1b]9;4;3\x1b\\")
        );
        assert_eq!(
            progress(&on, Progress::Complete).as_deref(),
            Some("\x1b]9;4;1;100\x1b\\")
        );
        assert_eq!(
            progress(&on, Progress::Clear).as_deref(),
            Some("\x1b]9;4;0\x1b\\")
        );
    }

    #[test]
    fn rich_notifications_are_encoded_and_never_doubled() {
        let rich = Capabilities::portable().with_verified(Verified {
            osc99_notifications: true,
            ..Verified::default()
        });
        let encoded = notification(&rich, "slow-1", "build", "done").unwrap();
        assert!(encoded.contains("\x1b]99;i=slow-1:d=0:e=1:p=title;YnVpbGQ="));
        assert!(encoded.contains("\x1b]99;i=slow-1:d=1:e=1:p=body;ZG9uZQ=="));
        assert!(!encoded.contains("\x1b]777;"));

        let unfocused = Capabilities::portable().with_verified(Verified {
            osc99_notifications: true,
            osc99_unfocused: true,
            ..Verified::default()
        });
        let encoded = notification(&unfocused, "slow-2", "build", "done").unwrap();
        assert_eq!(encoded.matches(":o=unfocused").count(), 2);
    }

    #[test]
    fn fallback_notification_cannot_inject_a_second_sequence() {
        let encoded = notification(
            &Capabilities::portable(),
            "ignored",
            "bad;title\x07",
            "line\nbody\x1b]0;owned",
        )
        .unwrap();
        assert_eq!(encoded, "\x1b]777;notify;bad,title;linebody]0,owned\x1b\\");
        assert_eq!(encoded.matches("\x1b]").count(), 1);
    }
}
