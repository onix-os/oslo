//! `ui format` — markdown, or a template, rendered for a terminal.
//!
//! Pure text in, pure text out: no terminal is touched, so it composes with pipes and can be
//! captured.
//!
//! # A deliberately small markdown
//!
//! Headings, bold, italic, inline code, fenced code, bullet and numbered lists, block quotes,
//! links and rules. That is the subset a `--help` text or a release note is written in, and it is
//! where the cost curve turns: nested lists, tables and reference links need a real parser, and a
//! shell that shipped one would be carrying a document engine to print help.
//!
//! Anything unrecognised passes through as its own text rather than being swallowed, so a document
//! this cannot render is still readable — which is the property that matters when the alternative
//! is a blank screen.

use crate::interactive::theme::{self, Color, Style};

/// What the input is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum As {
    Markdown,
    /// `{{name}}` replaced from the caller's pairs. gum calls this `template`.
    Template,
    /// Left exactly as it is, which is what makes `--type text` a way to turn formatting *off*.
    Text,
    /// Every line indented and dimmed, for showing a command's output inside a report.
    Code,
}

impl As {
    pub fn parse(name: &str) -> Option<As> {
        Some(match name {
            "markdown" | "md" => As::Markdown,
            "template" | "tmpl" => As::Template,
            "text" | "plain" => As::Text,
            "code" => As::Code,
            _ => return None,
        })
    }
}

/// Render `text`.
pub fn format(text: &str, kind: As, values: &[(String, String)]) -> String {
    match kind {
        As::Text => text.to_string(),
        As::Template => template(text, values),
        As::Code => code(text),
        As::Markdown => markdown(text),
    }
}

/// `{{name}}` from the caller's pairs; an unknown name is left as it was written.
///
/// Left rather than emptied on purpose: a template that silently loses a field looks like it
/// worked, and the mistake surfaces wherever the output is read instead of where it was made.
fn template(text: &str, values: &[(String, String)]) -> String {
    let mut out = text.to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{{{key}}}}}"), value);
    }
    out
}

fn code(text: &str) -> String {
    let ui = theme::current().ui;
    let depth = theme::depth();
    text.lines()
        .map(|line| format!("  {}", ui.muted.paint(line, depth)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown(text: &str) -> String {
    let theme = theme::current();
    let depth = theme::depth();
    let mut out = Vec::new();
    let mut fenced = false;
    let mut numbering = 0usize;

    for raw in text.lines() {
        let line = raw.trim_end();
        // A fence toggles verbatim mode, and everything inside is left alone — the point of a code
        // block is that its asterisks are asterisks.
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            out.push(format!("  {}", theme.ui.muted.paint(line, depth)));
            continue;
        }

        let trimmed = line.trim_start();
        let indent = &line[..line.len() - trimmed.len()];

        if let Some(rest) = heading(trimmed) {
            let (level, body) = rest;
            let style = Style {
                bold: true,
                ..Style::fg(if level == 1 {
                    theme.ui.accent.fg.unwrap_or(Color::Default)
                } else {
                    Color::Default
                })
            };
            out.push(String::new());
            out.push(style.paint(&inline(body, &theme, depth), depth));
            continue;
        }
        if is_rule(trimmed) {
            let width = crate::interactive::dropdown::width::terminal_cols().min(72);
            out.push(theme.ui.muted.paint(&"─".repeat(width), depth));
            continue;
        }
        if let Some(body) = trimmed.strip_prefix("> ") {
            out.push(format!(
                "{indent}{} {}",
                theme.ui.accent.paint("│", depth),
                theme.ui.muted.paint(&inline(body, &theme, depth), depth)
            ));
            continue;
        }
        if let Some(body) = bullet(trimmed) {
            numbering = 0;
            out.push(format!(
                "{indent}{} {}",
                theme.ui.accent.paint("•", depth),
                inline(body, &theme, depth)
            ));
            continue;
        }
        if let Some(body) = numbered(trimmed) {
            numbering += 1;
            out.push(format!(
                "{indent}{} {}",
                theme.ui.accent.paint(&format!("{numbering}."), depth),
                inline(body, &theme, depth)
            ));
            continue;
        }
        if trimmed.is_empty() {
            numbering = 0;
        }
        out.push(format!("{indent}{}", inline(trimmed, &theme, depth)));
    }
    out.join("\n")
}

/// `## Heading` — the level and the text.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.len() - line.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes, rest))
}

fn is_rule(line: &str) -> bool {
    line.len() >= 3 && (line.chars().all(|c| c == '-') || line.chars().all(|c| c == '*'))
}

fn bullet(line: &str) -> Option<&str> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))
}

fn numbered(line: &str) -> Option<&str> {
    let digits = line.len() - line.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return None;
    }
    line[digits..].strip_prefix(". ")
}

/// `**bold**`, `*italic*`, `` `code` `` and `[text](url)`, within one line.
fn inline(text: &str, theme: &theme::Theme, depth: theme::Depth) -> String {
    let mut out = String::new();
    let mut rest = text;
    while !rest.is_empty() {
        // Longest marker first, or `**bold**` is read as two italics around `*bold*`.
        if let Some(body) = between(rest, "**") {
            out.push_str(
                &Style {
                    bold: true,
                    ..Style::default()
                }
                .paint(body.0, depth),
            );
            rest = body.1;
            continue;
        }
        if let Some(body) = between(rest, "`") {
            out.push_str(&theme.ui.accent.paint(body.0, depth));
            rest = body.1;
            continue;
        }
        if let Some(body) = between(rest, "*") {
            out.push_str(
                &Style {
                    italic: true,
                    ..Style::default()
                }
                .paint(body.0, depth),
            );
            rest = body.1;
            continue;
        }
        if let Some((shown, url, after)) = link(rest) {
            out.push_str(shown);
            out.push(' ');
            out.push_str(&theme.ui.muted.paint(&format!("({url})"), depth));
            rest = after;
            continue;
        }
        let next = rest.char_indices().nth(1).map_or(rest.len(), |(i, _)| i);
        out.push_str(&rest[..next]);
        rest = &rest[next..];
    }
    out
}

/// The text between the next pair of `marker`s, and what follows the closing one.
fn between<'a>(text: &'a str, marker: &str) -> Option<(&'a str, &'a str)> {
    let body = text.strip_prefix(marker)?;
    let end = body.find(marker)?;
    Some((&body[..end], &body[end + marker.len()..]))
}

/// `[text](url)`.
fn link(text: &str) -> Option<(&str, &str, &str)> {
    let rest = text.strip_prefix('[')?;
    let close = rest.find(']')?;
    let after = rest[close + 1..].strip_prefix('(')?;
    let end = after.find(')')?;
    Some((&rest[..close], &after[..end], &after[end + 1..]))
}

#[cfg(test)]
#[path = "format/tests.rs"]
mod tests;
