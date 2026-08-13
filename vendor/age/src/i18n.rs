//! The strings age's errors are made of, in English.
//!
//! # What upstream does, and why it is not here
//!
//! Upstream loads these from Fluent files at runtime, through `i18n-embed`, `i18n-embed-fl` and
//! `rust-embed`. That is a localisation framework — thirty-odd crates including `fluent`,
//! `intl-memoizer`, `unic-langid`, `walkdir` and a TOML parser — carried so that "Decryption
//! failed" can be said in nine languages. In a shell that is also `/bin/sh` on a distribution, and
//! whose own diagnostics are English, none of it is reachable: nothing here calls `localizer()`, so
//! every one of those crates is linked to produce the same twenty-four English sentences that are
//! written out below.
//!
//! So the macros are the same three macros, with the same names and the same call shapes, and the
//! text is the same text — taken from `i18n/en-US/age.ftl`, which is kept beside this file so the
//! two can be compared. Adding a message means adding an arm; the compiler says so, because an
//! unmatched id is a macro that does not expand rather than a string that is missing at runtime.

/// One age message, by the id upstream gives it.
///
/// Each arm is one entry of `i18n/en-US/age.ftl`, with the same `{$name}` slots as named arguments.
#[doc(hidden)]
#[macro_export]
macro_rules! fl {
    ("err-decryption-failed") => {
        "Decryption failed".to_string()
    };
    ("err-excessive-work") => {
        "Excessive work parameter for passphrase.".to_string()
    };
    ("rec-excessive-work", duration = $duration:expr) => {
        format!("Decryption would take around {} seconds.", $duration)
    };
    ("err-failed-to-write-output", err = $err:expr) => {
        format!("Failed to write to output: {}", $err)
    };
    ("err-header-invalid") => {
        "Header is invalid".to_string()
    };
    ("err-header-mac-invalid") => {
        "Header MAC is invalid".to_string()
    };
    ("err-key-decryption") => {
        "Failed to decrypt an encrypted key".to_string()
    };
    ("err-no-matching-keys") => {
        "No matching keys found".to_string()
    };
    ("err-unknown-format") => {
        // `{-age}` in the catalogue: a Fluent term for the project's own name.
        "Unknown age format.".to_string()
    };
    ("rec-unknown-format") => {
        "Have you tried upgrading to the latest version?".to_string()
    };
    ("err-missing-recipients") => {
        "Missing recipients.".to_string()
    };
    ("err-mixed-recipient-passphrase") => {
        "A passphrase can't be used with other recipients.".to_string()
    };
    ("err-invalid-recipient-labels", labels = $labels:expr) => {
        format!(
            "The first recipient requires one or more invalid labels: '{}'",
            $labels
        )
    };
    ("err-incompatible-recipients-oneway", labels = $labels:expr) => {
        format!(
            "Cannot encrypt to a recipient with labels '{}' alongside a recipient with no labels",
            $labels
        )
    };
    ("err-incompatible-recipients-twoway", left = $left:expr, right = $right:expr) => {
        format!(
            "Cannot encrypt to a recipient with labels '{}' alongside a recipient with labels '{}'",
            $left, $right
        )
    };
    ("err-no-identities-in-file", filename = $filename:expr) => {
        format!("No identities found in file '{}'.", $filename)
    };
    ("err-no-identities-in-stdin") => {
        "No identities found in standard input.".to_string()
    };
    ("err-stream-last-chunk-empty") => {
        "Last STREAM chunk is empty. Please report this, and/or try an older rage version."
            .to_string()
    };
    ("encrypted-passphrase-prompt", filename = $filename:expr) => {
        format!("Type passphrase for encrypted identity '{}'", $filename)
    };
    ("encrypted-warn-no-match", filename = $filename:expr) => {
        format!(
            "Warning: encrypted identity file '{}' didn't match file's recipients",
            $filename
        )
    };
}

/// `write!`, with an age message.
#[doc(hidden)]
#[macro_export]
macro_rules! wfl {
    ($f:ident, $message_id:tt) => {
        write!($f, "{}", $crate::fl!($message_id))
    };

    ($f:ident, $message_id:tt, $($name:ident = $value:expr),* $(,)?) => {
        write!($f, "{}", $crate::fl!($message_id, $($name = $value),*))
    };
}

/// `writeln!`, with an age message.
#[doc(hidden)]
#[macro_export]
macro_rules! wlnfl {
    ($f:ident, $message_id:tt) => {
        writeln!($f, "{}", $crate::fl!($message_id))
    };

    ($f:ident, $message_id:tt, $($name:ident = $value:expr),* $(,)?) => {
        writeln!($f, "{}", $crate::fl!($message_id, $($name = $value),*))
    };
}
