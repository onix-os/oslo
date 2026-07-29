//! Turning a parsed function body back into shell source.
//!
//! `set` with no arguments lists function definitions alongside variables, and the whole value of
//! that listing is that it can be read back — so the AST has to be printed as source, not as
//! `{:?}`. Nothing else in rush needs this, which is why it lives next to the listing builtins
//! rather than in [`crate::ast`].
//!
//! One deliberate simplification: everything below the definition's outermost braces is printed
//! on a single line, with `;` where a newline would have been. `if x; then y; fi` is the same
//! program as the four-line form, and one line per top-level statement keeps the printer to a
//! string builder instead of an indentation engine.

use crate::ast::{
    AndOrList, AndOrOp, Assignment, AssignmentTarget, AssignmentValue, Command, CommandList,
    CompoundCommand, ListOp, ParamExpansion, Pipeline, RedirectKind, Redirection, ReplaceScope,
    SimpleCommand, Subscript, Word, WordPart,
};

/// Render `name` and `body` as a definition a shell can parse again.
pub fn function_definition(name: &str, body: &Command) -> String {
    let mut d = Deparser::default();
    d.out.push_str(name);
    d.out.push_str(" ()\n");
    d.out.push_str("{\n");
    match body {
        // The usual shape: `f() { ... }` parses to a group, whose statements become the lines of
        // the definition. Anything else — a single compound, a bare pipeline — is one line.
        Command::Compound {
            kind: CompoundCommand::Group(list),
            redirections,
        } if redirections.is_empty() => {
            for item in &list.items {
                let text = d.item(item.op, &item.and_or);
                d.emit(&text);
            }
        }
        other => {
            let text = d.command(other);
            d.emit(&text);
        }
    }
    d.out.push_str("}\n");
    d.out
}

#[derive(Default)]
struct Deparser {
    out: String,
    /// Heredoc bodies belonging to the line being built, flushed once it is emitted.
    ///
    /// A heredoc cannot be folded onto one line the way `;` folds a list, so the redirection
    /// operator goes in the line and the body follows it — which is exactly where the shell
    /// looks for it, however deeply the redirection was nested in the command.
    pending: Vec<String>,
    /// Counter behind the generated delimiters, so two heredocs on one line cannot collide.
    seq: usize,
}

impl Deparser {
    /// Write one indented line, then any heredoc bodies it owes, which must start at column 0.
    fn emit(&mut self, text: &str) {
        self.out.push_str("    ");
        self.out.push_str(text);
        self.out.push('\n');
        for body in std::mem::take(&mut self.pending) {
            self.out.push_str(&body);
        }
    }

    fn item(&mut self, op: ListOp, and_or: &AndOrList) -> String {
        let text = self.and_or(and_or);
        match op {
            ListOp::Background => format!("{} &", text),
            _ => text,
        }
    }

    /// A command list folded onto one line: `a; b & c`.
    fn list(&mut self, list: &CommandList) -> String {
        let mut out = String::new();
        for (idx, item) in list.items.iter().enumerate() {
            if idx > 0 {
                out.push(' ');
            }
            out.push_str(&self.item(item.op, &item.and_or));
            if idx + 1 < list.items.len() && !matches!(item.op, ListOp::Background) {
                out.push(';');
            }
        }
        out
    }

    /// A list plus the terminator a keyword needs after it: `cond;` in `if cond; then`.
    ///
    /// `&` already terminates a list, and `cmd &;` is a syntax error, so it is left alone.
    fn terminated(&mut self, list: &CommandList) -> String {
        let text = self.list(list);
        if text.ends_with('&') {
            text
        } else {
            format!("{};", text)
        }
    }

    fn and_or(&mut self, and_or: &AndOrList) -> String {
        let mut out = self.pipeline(&and_or.first);
        for (op, pipeline) in &and_or.rest {
            let op = match op {
                AndOrOp::And => "&&",
                AndOrOp::Or => "||",
            };
            let rhs = self.pipeline(pipeline);
            out.push_str(&format!(" {} {}", op, rhs));
        }
        out
    }

    fn pipeline(&mut self, pipeline: &Pipeline) -> String {
        let stages: Vec<String> = pipeline
            .commands
            .iter()
            .map(|cmd| self.command(cmd))
            .collect();
        let joined = stages.join(" | ");
        if pipeline.negated {
            format!("! {}", joined)
        } else {
            joined
        }
    }

    fn command(&mut self, cmd: &Command) -> String {
        match cmd {
            Command::Simple(simple) => self.simple(simple),
            Command::Compound { kind, redirections } => {
                let mut text = self.compound(kind);
                for redirection in redirections {
                    let rendered = self.redirection(redirection);
                    text.push(' ');
                    text.push_str(&rendered);
                }
                text
            }
            Command::FunctionDef { name, body } => {
                let body = self.command(body);
                format!("{} () {}", name, body)
            }
        }
    }

    fn compound(&mut self, kind: &CompoundCommand) -> String {
        match kind {
            CompoundCommand::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            } => {
                let mut parts = vec!["if".to_string(), self.terminated(condition)];
                parts.push("then".to_string());
                parts.push(self.terminated(then_branch));
                for (cond, body) in elif_branches {
                    parts.push("elif".to_string());
                    parts.push(self.terminated(cond));
                    parts.push("then".to_string());
                    parts.push(self.terminated(body));
                }
                if let Some(body) = else_branch {
                    parts.push("else".to_string());
                    parts.push(self.terminated(body));
                }
                parts.push("fi".to_string());
                parts.join(" ")
            }
            CompoundCommand::While { condition, body } => {
                let cond = self.terminated(condition);
                let body = self.terminated(body);
                format!("while {} do {} done", cond, body)
            }
            CompoundCommand::Until { condition, body } => {
                let cond = self.terminated(condition);
                let body = self.terminated(body);
                format!("until {} do {} done", cond, body)
            }
            CompoundCommand::For {
                var_name,
                items,
                body,
            } => {
                let head = match items {
                    Some(words) => {
                        let rendered: Vec<String> = words.iter().map(word).collect();
                        format!("for {} in {};", var_name, rendered.join(" "))
                    }
                    // `for v; do` iterates the positional parameters; `in` with nothing after it
                    // would iterate an empty list instead, which is a different program.
                    None => format!("for {};", var_name),
                };
                let body = self.terminated(body);
                format!("{} do {} done", head, body)
            }
            CompoundCommand::Case {
                word: subject,
                items,
            } => {
                let mut parts = vec![format!("case {} in", word(subject))];
                for item in items {
                    let patterns: Vec<String> = item.patterns.iter().map(word).collect();
                    let body = self.list(&item.body);
                    // The branch's own terminator, not a blanket `;;`: `;&` and `;;&` are
                    // different programs, and this text has to re-parse to the same tree.
                    let end = item.post_action.terminator();
                    if body.is_empty() {
                        parts.push(format!("{}) {end}", patterns.join("|")));
                    } else {
                        parts.push(format!("{}) {}{end}", patterns.join("|"), body));
                    }
                }
                parts.push("esac".to_string());
                parts.join(" ")
            }
            CompoundCommand::Arithmetic(expr) => format!("(( {} ))", expr),
            CompoundCommand::ArithmeticFor {
                init,
                cond,
                step,
                body,
            } => {
                let section = |e: &Option<String>| e.clone().unwrap_or_default();
                let body = self.terminated(body);
                format!(
                    "for (( {}; {}; {} )); do {} done",
                    section(init),
                    section(cond),
                    section(step),
                    body
                )
            }
            CompoundCommand::Subshell(list) => {
                let body = self.list(list);
                format!("({})", body)
            }
            CompoundCommand::Group(list) => {
                let body = self.terminated(list);
                format!("{{ {} }}", body)
            }
        }
    }

    fn simple(&mut self, simple: &SimpleCommand) -> String {
        let mut parts: Vec<String> = simple.assignments.iter().map(assignment).collect();
        parts.extend(simple.words.iter().map(word));
        for redirection in &simple.redirections {
            let rendered = self.redirection(redirection);
            parts.push(rendered);
        }
        parts.join(" ")
    }

    fn redirection(&mut self, redirection: &Redirection) -> String {
        let fd = redirection.fd.map(|n| n.to_string()).unwrap_or_default();
        let op = match redirection.kind {
            RedirectKind::Input => "<",
            RedirectKind::Output => ">",
            RedirectKind::Append => ">>",
            RedirectKind::ReadWrite => "<>",
            RedirectKind::DupInput => "<&",
            RedirectKind::DupOutput => ">&",
            RedirectKind::Heredoc => "<<",
            RedirectKind::HeredocStrip => "<<-",
            RedirectKind::Clobber => ">|",
        };
        match redirection.kind {
            RedirectKind::Heredoc | RedirectKind::HeredocStrip => {
                self.seq += 1;
                let delimiter = format!("RUSH_HEREDOC_{}", self.seq);
                let mut body = redirection.heredoc_content.clone().unwrap_or_default();
                if !body.is_empty() && !body.ends_with('\n') {
                    body.push('\n');
                }
                body.push_str(&delimiter);
                body.push('\n');
                self.pending.push(body);
                // The delimiter is left unquoted because the AST does not record whether the
                // original one was quoted, and an unquoted heredoc — the common case — must keep
                // expanding its body when the definition is read back.
                format!("{}{}{}", fd, op, delimiter)
            }
            _ => format!("{}{}{}", fd, op, word(&redirection.target)),
        }
    }
}

/// Render a word outside quotes.
fn word(w: &Word) -> String {
    w.parts.iter().map(|p| part(p, false)).collect()
}

/// Render one word part. `in_quotes` selects the escaping rules of a double-quoted context.
fn part(p: &WordPart, in_quotes: bool) -> String {
    match p {
        WordPart::Literal(s) if in_quotes => s
            .chars()
            .map(|c| match c {
                '"' | '\\' | '$' | '`' => format!("\\{}", c),
                c => c.to_string(),
            })
            .collect(),
        WordPart::Literal(s) => s.clone(),
        WordPart::Escaped(s) => s.chars().map(|c| format!("\\{}", c)).collect(),
        WordPart::SingleQuoted(s) => format!("'{}'", s),
        WordPart::DoubleQuoted(parts) => {
            let inner: String = parts.iter().map(|p| part(p, true)).collect();
            format!("\"{}\"", inner)
        }
        WordPart::Variable {
            name,
            expansion_type,
        } => parameter(name, expansion_type),
        // Always braced, like every other reference here: `$a[0]` would read back as `$a`
        // followed by the literal `[0]`.
        WordPart::ArrayRef {
            name,
            subscript,
            expansion_type,
        } => parameter(
            &format!("{}[{}]", name, subscript_text(subscript)),
            expansion_type,
        ),
        WordPart::CommandSubstitution(s) => format!("$({})", s),
        WordPart::Arithmetic(s) => format!("$(({}))", s),
        WordPart::Tilde(s) => format!("~{}", s),
    }
}

/// Render an assignment, in whichever of its four shapes it has.
fn assignment(a: &Assignment) -> String {
    let target = match &a.target {
        AssignmentTarget::Name(name) => name.clone(),
        AssignmentTarget::Element { name, index } => format!("{}[{}]", name, word(index)),
    };
    let op = if a.append { "+=" } else { "=" };
    let value = match &a.value {
        AssignmentValue::Scalar(w) => word(w),
        AssignmentValue::Array(elements) => {
            let rendered: Vec<String> = elements
                .iter()
                .map(|e| match &e.index {
                    Some(index) => format!("[{}]={}", word(index), word(&e.value)),
                    None => word(&e.value),
                })
                .collect();
            format!("({})", rendered.join(" "))
        }
    };
    format!("{}{}{}", target, op, value)
}

fn subscript_text(subscript: &Subscript) -> String {
    match subscript {
        Subscript::All => "@".to_string(),
        Subscript::Joined => "*".to_string(),
        Subscript::Index(w) => word(w),
    }
}

/// Render `${name...}`. Always braced: `$1x` and `${1}x` are different parameters.
fn parameter(name: &str, expansion: &ParamExpansion) -> String {
    let colon = |test_null: &bool| if *test_null { ":" } else { "" };
    match expansion {
        ParamExpansion::Normal => format!("${{{}}}", name),
        ParamExpansion::Length => format!("${{#{}}}", name),
        ParamExpansion::Indirect => format!("${{!{}}}", name),
        ParamExpansion::DefaultValue {
            default,
            assign_if_unset,
            test_null,
        } => {
            let op = if *assign_if_unset { "=" } else { "-" };
            format!("${{{}{}{}{}}}", name, colon(test_null), op, word(default))
        }
        ParamExpansion::UseAlternative {
            alternative,
            test_null,
        } => format!("${{{}{}+{}}}", name, colon(test_null), word(alternative)),
        ParamExpansion::ErrorIfUnset { message, test_null } => {
            format!("${{{}{}?{}}}", name, colon(test_null), word(message))
        }
        ParamExpansion::RemoveSuffix { pattern, longest } => {
            let op = if *longest { "%%" } else { "%" };
            format!("${{{}{}{}}}", name, op, word(pattern))
        }
        ParamExpansion::RemovePrefix { pattern, longest } => {
            let op = if *longest { "##" } else { "#" };
            format!("${{{}{}{}}}", name, op, word(pattern))
        }
        ParamExpansion::Substring { offset, length } => match length {
            Some(len) => format!("${{{}:{}:{}}}", name, word(offset), word(len)),
            None => format!("${{{}:{}}}", name, word(offset)),
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
            format!("${{{}{}{}/{}}}", name, op, word(pattern), word(replacement))
        }
        ParamExpansion::CaseConvert {
            pattern,
            upper,
            all,
        } => {
            let op = match (upper, all) {
                (true, true) => "^^",
                (true, false) => "^",
                (false, true) => ",,",
                (false, false) => ",",
            };
            let pattern = pattern.as_ref().map(word).unwrap_or_default();
            format!("${{{}{}{}}}", name, op, pattern)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::function_definition;
    use crate::ast::Command;
    use crate::parser::parse_bash_script;

    /// Parse `src`, which must define exactly one function, and print it back.
    fn round_trip(src: &str) -> String {
        let ast = parse_bash_script(src).expect("the source parses");
        for item in &ast.items {
            for cmd in &item.and_or.first.commands {
                if let Command::FunctionDef { name, body } = cmd {
                    return function_definition(name, body);
                }
            }
        }
        panic!("no function definition in {src:?}");
    }

    /// The printed definition must parse again, and print the same thing the second time.
    fn is_stable(src: &str) {
        let once = round_trip(src);
        let twice = round_trip(&once);
        assert_eq!(once, twice, "printing {src:?} is not stable");
    }

    #[test]
    fn a_simple_body_round_trips() {
        assert_eq!(round_trip("f() { echo hi; }"), "f ()\n{\n    echo hi\n}\n");
        is_stable("f() { echo hi; }");
    }

    #[test]
    fn control_flow_folds_onto_one_line() {
        is_stable("f() { if true; then echo a; else echo b; fi; }");
        is_stable("f() { while read x; do echo $x; done; }");
        is_stable("f() { for i in a b c; do echo $i; done; }");
        is_stable("f() { case $1 in a|b) echo ab;; *) echo other;; esac; }");
    }

    #[test]
    fn quoting_and_expansions_survive() {
        is_stable(r#"f() { echo "a $x b" '$y' ${z:-d}; }"#);
        is_stable("f() { echo ${#a} ${b#pre} ${c%%suf}; }");
    }

    #[test]
    fn redirections_and_pipelines_survive() {
        is_stable("f() { cat < in > out 2>&1; }");
        is_stable("f() { ! a | b && c || d; }");
    }

    /// A heredoc cannot live on the folded line, so its body follows the line it belongs to.
    #[test]
    fn a_heredoc_body_follows_its_line() {
        let printed = round_trip("f() {\ncat <<EOF\nbody\nEOF\n}");
        assert!(printed.contains("<<RUSH_HEREDOC_1"), "{printed}");
        assert!(printed.contains("\nbody\nRUSH_HEREDOC_1\n"), "{printed}");
        is_stable("f() {\ncat <<EOF\nbody\nEOF\n}");
    }
}
