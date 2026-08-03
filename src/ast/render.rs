//! Turning a parsed command back into shell text.
//!
//! This exists for one caller: `$BASH_COMMAND`, which the DEBUG trap sets to the command about to
//! run. Every preexec integration reads it — hexe's `__hexe_preexec` compares it against its own
//! function names to avoid recording its own hooks, and bash-preexec falls back to it — so the
//! text has to be recognisable, not merely present.
//!
//! **It is a re-render, not the source.** bash does the same thing and it shows: given
//! `ls /x 2>/dev/null`, bash's own `$BASH_COMMAND` reads `ls /x 2> /dev/null`, with a space the
//! script never wrote. Quoting is normalised the same way here. Anything that needs the text
//! exactly as typed wants the history entry, not this.
//!
//! Only [`SimpleCommand`] is rendered, because only simple commands fire the DEBUG trap.

use super::types::*;
use std::fmt::Write;

/// A simple command as shell text, close enough to the source to be recognised.
pub fn simple_command(cmd: &SimpleCommand) -> String {
    let mut out = String::new();
    for assign in &cmd.assignments {
        push_space(&mut out);
        out.push_str(&assignment(assign));
    }
    for word in &cmd.words {
        push_space(&mut out);
        out.push_str(&word_text(word));
    }
    for redirect in &cmd.redirections {
        push_space(&mut out);
        out.push_str(&redirection(redirect));
    }
    out
}

fn push_space(out: &mut String) {
    if !out.is_empty() {
        out.push(' ');
    }
}

fn assignment(assign: &Assignment) -> String {
    let target = match &assign.target {
        AssignmentTarget::Name(name) => name.clone(),
        AssignmentTarget::Element { name, index } => format!("{name}[{}]", word_text(index)),
    };
    let op = if assign.append { "+=" } else { "=" };
    let value = match &assign.value {
        AssignmentValue::Scalar(word) => word_text(word),
        AssignmentValue::Array(elements) => {
            let parts: Vec<String> = elements
                .iter()
                .map(|e| match &e.index {
                    Some(index) => format!("[{}]={}", word_text(index), word_text(&e.value)),
                    None => word_text(&e.value),
                })
                .collect();
            format!("({})", parts.join(" "))
        }
    };
    format!("{target}{op}{value}")
}

/// `2> /dev/null`, with bash's space after the operator.
fn redirection(redirect: &Redirection) -> String {
    let mut out = String::new();
    if let Some(fd) = redirect.fd {
        let _ = write!(out, "{fd}");
    }
    out.push_str(operator(&redirect.kind, redirect.here_string));
    out.push(' ');
    // A here-document's operand is its delimiter, which the parser has already consumed along with
    // the body. Rendering the body in its place would be a lie the length of the document, so the
    // delimiter word is what goes here — exactly what the source said.
    out.push_str(&word_text(&redirect.target));
    out
}

fn operator(kind: &RedirectKind, here_string: bool) -> &'static str {
    match kind {
        RedirectKind::Input => "<",
        RedirectKind::Output => ">",
        RedirectKind::Append => ">>",
        RedirectKind::Clobber => ">|",
        RedirectKind::ReadWrite => "<>",
        RedirectKind::Heredoc | RedirectKind::HeredocStrip if here_string => "<<<",
        RedirectKind::Heredoc => "<<",
        RedirectKind::HeredocStrip => "<<-",
        RedirectKind::DupInput => "<&",
        RedirectKind::DupOutput => ">&",
    }
}

/// One word, with its quoting put back.
pub fn word_text(word: &Word) -> String {
    word.parts.iter().map(part_text).collect()
}

fn part_text(part: &WordPart) -> String {
    match part {
        WordPart::Literal(text) => text.clone(),
        // The backslash was stripped at lex time and the character means itself; putting the
        // backslash back is what keeps the render re-parsable.
        WordPart::Escaped(text) => text.chars().map(|c| format!("\\{c}")).collect(),
        WordPart::SingleQuoted(text) => format!("'{text}'"),
        WordPart::DoubleQuoted(parts) => {
            let inner: String = parts.iter().map(part_text).collect();
            format!("\"{inner}\"")
        }
        WordPart::Variable {
            name,
            expansion_type,
        } => variable(name, expansion_type),
        WordPart::ArrayRef {
            name,
            subscript,
            expansion_type,
        } => {
            let index = match subscript {
                Subscript::All => "@".to_string(),
                Subscript::Joined => "*".to_string(),
                Subscript::Index(word) => word_text(word),
            };
            variable(&format!("{name}[{index}]"), expansion_type)
        }
        WordPart::CommandSubstitution(text) => format!("$({text})"),
        WordPart::ProcessSubstitution {
            reads_from_command,
            command,
        } => {
            let open = if *reads_from_command { '<' } else { '>' };
            format!("{open}({command})")
        }
        WordPart::Arithmetic(text) => format!("$(({text}))"),
        WordPart::Tilde(text) => format!("~{text}"),
    }
}

/// `$x`, `${x}`, or `${x<op>word}` — whichever spelling the expansion needs.
fn variable(name: &str, expansion: &ParamExpansion) -> String {
    let body = match expansion {
        // A bare name needs no braces, and `$PATH` reads better than `${PATH}` in a hook's
        // diagnostic. A name that is not a plain identifier — `${a[0]}`, `${11}` — keeps them.
        ParamExpansion::Normal => {
            return if plain_name(name) {
                format!("${name}")
            } else {
                format!("${{{name}}}")
            };
        }
        ParamExpansion::Length => format!("#{name}"),
        ParamExpansion::Indirect => format!("!{name}"),
        ParamExpansion::DefaultValue {
            default,
            assign_if_unset,
            test_null,
        } => {
            let op = if *assign_if_unset { "=" } else { "-" };
            format!("{name}{}{op}{}", colon(*test_null), word_text(default))
        }
        ParamExpansion::UseAlternative {
            alternative,
            test_null,
        } => format!("{name}{}+{}", colon(*test_null), word_text(alternative)),
        ParamExpansion::ErrorIfUnset { message, test_null } => {
            format!("{name}{}?{}", colon(*test_null), word_text(message))
        }
        ParamExpansion::RemoveSuffix { pattern, longest } => {
            format!("{name}{}{}", repeat('%', *longest), word_text(pattern))
        }
        ParamExpansion::RemovePrefix { pattern, longest } => {
            format!("{name}{}{}", repeat('#', *longest), word_text(pattern))
        }
        ParamExpansion::Substring { offset, length } => match length {
            Some(length) => format!("{name}:{}:{}", word_text(offset), word_text(length)),
            None => format!("{name}:{}", word_text(offset)),
        },
        ParamExpansion::Replace {
            pattern,
            replacement,
            scope,
        } => {
            let op = match scope {
                ReplaceScope::First => "/",
                ReplaceScope::All => "//",
                ReplaceScope::Prefix => "/#",
                ReplaceScope::Suffix => "/%",
            };
            format!(
                "{name}{op}{}/{}",
                word_text(pattern),
                word_text(replacement)
            )
        }
        ParamExpansion::CaseConvert {
            pattern,
            upper,
            all,
        } => {
            let op = repeat(if *upper { '^' } else { ',' }, *all);
            let pattern = pattern.as_ref().map(word_text).unwrap_or_default();
            format!("{name}{op}{pattern}")
        }
    };
    format!("${{{body}}}")
}

fn colon(test_null: bool) -> &'static str {
    if test_null { ":" } else { "" }
}

fn repeat(c: char, twice: bool) -> String {
    if twice {
        format!("{c}{c}")
    } else {
        c.to_string()
    }
}

/// Whether `$name` alone is an unambiguous way to write this reference.
fn plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        // The special parameters are each one character and each unambiguous bare.
        Some(c) if "@*#?-$!0".contains(c) => return name.chars().count() == 1,
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;
