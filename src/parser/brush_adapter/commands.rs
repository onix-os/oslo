//! Command and compound-command conversion.
//!
//! Turns brush's `Command` and `CompoundCommand` nodes into rush's equivalents: simple
//! commands with their assignments and redirections, `if`/`while`/`for`/`case` bodies, and
//! function definitions.

use super::convert_compound_list;
use super::extended_test::convert_extended_test;
use super::redirects::{convert_redirect, convert_redirect_list};
use super::words::{
    convert_braced_word, convert_braced_words_from_str, convert_word, convert_words_from_str,
    single_command_list, single_word,
};
use crate::ast as rush_ast;
use crate::error::{Result, ShellError};
use brush_parser::ast;

pub(super) fn convert_command(cmd: &ast::Command) -> Result<rush_ast::Command> {
    match cmd {
        ast::Command::Simple(simple) => {
            Ok(rush_ast::Command::Simple(convert_simple_command(simple)?))
        }
        ast::Command::Compound(compound, redirects) => {
            let mut converted = convert_compound_command(compound)?;
            // Redirections attached to the whole compound, e.g. `while ...; done > log`.
            if let (rush_ast::Command::Compound { redirections, .. }, Some(list)) =
                (&mut converted, redirects)
            {
                redirections.extend(convert_redirect_list(list)?);
            }
            Ok(converted)
        }
        ast::Command::Function(func) => {
            let mut body = convert_compound_command(&func.body.0)?;
            if let (rush_ast::Command::Compound { redirections, .. }, Some(list)) =
                (&mut body, &func.body.1)
            {
                redirections.extend(convert_redirect_list(list)?);
            }
            Ok(rush_ast::Command::FunctionDef {
                name: func.fname.to_string(),
                body: Box::new(body),
            })
        }
        ast::Command::ExtendedTest(expr, redirects) => {
            let converted = convert_extended_test(&expr.expr)?;
            // `[[ ... ]] > file` is legal but vanishingly rare; wrap in a group so the
            // redirection has somewhere to live.
            match redirects {
                None => Ok(converted),
                Some(list) => Ok(rush_ast::Command::Compound {
                    kind: rush_ast::CompoundCommand::Group(single_command_list(converted)),
                    redirections: convert_redirect_list(list)?,
                }),
            }
        }
    }
}

/// A construct rush parses but cannot yet execute.
///
/// Reported as a syntax error so it reaches the user with a non-zero status and the name of the
/// thing that is missing, rather than being approximated into something that runs.
fn unsupported(construct: &str) -> ShellError {
    ShellError::SyntaxError(format!("{} is not supported yet", construct))
}

/// A construct rush's parser cannot even *reach*, recognised from the source text.
///
/// `select` is absent from brush 0.4's grammar entirely, so it surfaces as a bare "syntax error
/// at line N col M" pointing at the `in` — a diagnostic that reads like a typo in the user's
/// script rather than a gap in the shell. Callers that hold a parse error can offer this instead:
/// [`ShellError::SyntaxError`] naming `select`, with the same non-zero status.
///
/// Deliberately conservative. It only fires when a line *begins* with the keyword followed by a
/// name, so `echo select` and `x=select` are untouched; a false positive here would replace a
/// truthful diagnostic with a misleading one.
pub(super) fn unreachable_construct(script: &str) -> Option<ShellError> {
    let names_a_variable = |rest: &str| {
        let name = rest.trim_start();
        let head = name
            .split([' ', '\t', ';', '\n'])
            .next()
            .unwrap_or_default();
        !head.is_empty()
            && head.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && head.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };

    script
        .lines()
        .map(|line| line.trim_start())
        .filter_map(|line| line.strip_prefix("select"))
        .filter(|rest| rest.starts_with([' ', '\t']))
        .find(|rest| names_a_variable(rest))
        .map(|_| unsupported("select"))
}

pub(super) fn convert_compound_command(
    compound: &ast::CompoundCommand,
) -> Result<rush_ast::Command> {
    let kind = match compound {
        ast::CompoundCommand::BraceGroup(group) => {
            rush_ast::CompoundCommand::Group(convert_compound_list(&group.list)?)
        }
        ast::CompoundCommand::Subshell(subshell) => {
            rush_ast::CompoundCommand::Subshell(convert_compound_list(&subshell.list)?)
        }
        ast::CompoundCommand::IfClause(if_clause) => convert_if_clause(if_clause)?,
        ast::CompoundCommand::WhileClause(while_clause) => rush_ast::CompoundCommand::While {
            condition: convert_compound_list(&while_clause.0)?,
            body: convert_compound_list(&while_clause.1.list)?,
        },
        ast::CompoundCommand::UntilClause(until_clause) => rush_ast::CompoundCommand::Until {
            condition: convert_compound_list(&until_clause.0)?,
            body: convert_compound_list(&until_clause.1.list)?,
        },
        ast::CompoundCommand::ForClause(for_clause) => {
            let items = match &for_clause.values {
                Some(vals) => {
                    let mut converted = Vec::new();
                    for w in vals {
                        converted.extend(convert_braced_word(w)?);
                    }
                    Some(converted)
                }
                None => None,
            };
            rush_ast::CompoundCommand::For {
                var_name: for_clause.variable_name.to_string(),
                items,
                body: convert_compound_list(&for_clause.body.list)?,
            }
        }
        ast::CompoundCommand::CaseClause(case_clause) => convert_case_clause(case_clause)?,
        // The expression text is carried through unparsed on purpose: parameters and command
        // substitutions inside it are expanded when the command runs, not now.
        ast::CompoundCommand::Arithmetic(arith) => {
            rush_ast::CompoundCommand::Arithmetic(arith.expr.value.clone())
        }
        ast::CompoundCommand::ArithmeticForClause(for_clause) => {
            rush_ast::CompoundCommand::ArithmeticFor {
                init: expr_text(for_clause.initializer.as_ref()),
                cond: expr_text(for_clause.condition.as_ref()),
                step: expr_text(for_clause.updater.as_ref()),
                body: convert_compound_list(&for_clause.body.list)?,
            }
        }
        // `coproc` parses but has no representation in rush's AST, and cannot get one until there
        // are arrays to publish the descriptors in and job control to manage the child. Naming
        // the construct matters: this diagnostic is the whole of the user's feedback, where it
        // used to be swallowed by a fallback parser that ran the body inline and synchronously.
        ast::CompoundCommand::Coprocess(_) => return Err(unsupported("coproc")),
    };

    Ok(rush_ast::Command::Compound {
        kind,
        redirections: Vec::new(),
    })
}

/// Convert an `if`/`elif`/`else` chain.
///
/// brush flattens the chain into `elses: Vec<ElseClause>`, where a clause carrying a condition is
/// an `elif` and the (at most one) clause without is the final `else`. Both map directly onto
/// rush's `elif_branches` / `else_branch`.
fn convert_if_clause(if_clause: &ast::IfClauseCommand) -> Result<rush_ast::CompoundCommand> {
    let condition = convert_compound_list(&if_clause.condition)?;
    let then_branch = convert_compound_list(&if_clause.then)?;

    let mut elif_branches = Vec::new();
    let mut else_branch = None;

    if let Some(elses) = &if_clause.elses {
        for clause in elses {
            match &clause.condition {
                Some(cond) => elif_branches.push((
                    convert_compound_list(cond)?,
                    convert_compound_list(&clause.body)?,
                )),
                None => else_branch = Some(convert_compound_list(&clause.body)?),
            }
        }
    }

    Ok(rush_ast::CompoundCommand::If {
        condition,
        then_branch,
        elif_branches,
        else_branch,
    })
}

fn convert_case_clause(case_clause: &ast::CaseClauseCommand) -> Result<rush_ast::CompoundCommand> {
    let word = single_word(&case_clause.value)?;

    let mut items = Vec::new();
    for case in &case_clause.cases {
        let mut patterns = Vec::new();
        for p in &case.patterns {
            patterns.extend(convert_word(p)?);
        }

        let body = match &case.cmd {
            Some(list) => convert_compound_list(list)?,
            None => rush_ast::CommandList::default(),
        };

        // The terminator is part of the program, not punctuation: `;&` and `;;&` select different
        // branches from `;;`, and collapsing all three made a fallthrough chain run one branch.
        let post_action = match case.post_action {
            ast::CaseItemPostAction::ExitCase => rush_ast::CaseAction::ExitCase,
            ast::CaseItemPostAction::UnconditionallyExecuteNextCaseItem => {
                rush_ast::CaseAction::FallThrough
            }
            ast::CaseItemPostAction::ContinueEvaluatingCases => {
                rush_ast::CaseAction::ContinueMatching
            }
        };

        items.push(rush_ast::CaseItem {
            patterns,
            body,
            post_action,
        });
    }

    Ok(rush_ast::CompoundCommand::Case { word, items })
}

/// brush stores each section of a `for ((…))` head as raw, unexpanded text; rush keeps it that
/// way so the shell's own expansions run over it at loop time.
///
/// A section holding nothing but space is *absent*, not "the expression 0": brush hands back
/// `Some(" ")` for the middle of `for (( ; ; ))`, and evaluating that as a condition would end
/// the loop before its first iteration instead of running it forever.
///
/// Note that brush 0.4 cannot parse the unspaced `for ((;;))` at all — its tokenizer reads `;;`
/// as the `case` terminator, and the grammar wants two `;` operators.
fn expr_text(expr: Option<&ast::UnexpandedArithmeticExpr>) -> Option<String> {
    expr.map(|e| e.value.clone())
        .filter(|text| !text.trim().is_empty())
}

/// `<(cmd)` / `>(cmd)` used as a *word*.
///
/// The same error the redirect-target form has always raised. Until this file returned it, a
/// process substitution in argument position was deleted from argv and nothing said so: `cat
/// <(echo hi)` ran `cat` with no arguments and exited 0, and `diff <(a) <(b)` reported a false
/// success — a wrong answer with a passing status, which is the worst shape a shell bug can take.
/// The `/dev/fd/N` implementation is deferred; refusing is what makes the gap visible.
fn process_substitution_unsupported() -> ShellError {
    ShellError::SyntaxError("process substitution is not supported".to_string())
}

/// Convert `[[ ... ]]` into equivalent `test` commands.
///
/// rush has no dedicated node for extended tests, but the expression tree maps cleanly onto
/// constructs it already evaluates: `&&` and `||` become an and-or list, `!` becomes a negated
/// pipeline, and the leaf predicates become `test` invocations handled by the existing builtin.
///
/// This is a real (if partial) evaluation — the previous behaviour was to return the literal
/// command `true`, so every `[[ ... ]]` succeeded regardless of its contents.
pub(super) fn convert_simple_command(
    simple: &ast::SimpleCommand,
) -> Result<rush_ast::SimpleCommand> {
    let mut words = Vec::new();
    let mut assignments = Vec::new();
    let mut redirections = Vec::new();

    if let Some(prefix) = &simple.prefix {
        for item in &prefix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::AssignmentWord(assign, _) => {
                    assignments.push(convert_assignment(assign)?);
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
                    redirections.extend(convert_redirect(redir)?);
                }
                // brush's prefix grammar admits only assignments and redirections, but the item
                // enum is shared with the suffix. If one ever arrives it is an argument, and
                // dropping it would change the command being run.
                ast::CommandPrefixOrSuffixItem::Word(w) => {
                    words.extend(convert_braced_word(w)?);
                }
                ast::CommandPrefixOrSuffixItem::ProcessSubstitution(..) => {
                    return Err(process_substitution_unsupported());
                }
            }
        }
    }

    if let Some(cmd_word) = &simple.word_or_name {
        words.extend(convert_braced_word(cmd_word)?);
    }

    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::Word(w) => {
                    words.extend(convert_braced_word(w)?);
                }
                ast::CommandPrefixOrSuffixItem::AssignmentWord(_, raw) => {
                    // Only a *prefix* `name=value` is an assignment; after the command name it
                    // is an ordinary argument (`alias g='echo hi'`, `env FOO=bar`).
                    //
                    // Use brush's raw source word rather than re-joining name and value: the
                    // rejoined text would be a bare literal, so its quotes would survive
                    // expansion and the value would then be field-split on its own spaces.
                    words.extend(convert_braced_word(raw)?);
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
                    redirections.extend(convert_redirect(redir)?);
                }
                ast::CommandPrefixOrSuffixItem::ProcessSubstitution(..) => {
                    return Err(process_substitution_unsupported());
                }
            }
        }
    }

    Ok(rush_ast::SimpleCommand {
        assignments,
        words,
        redirections,
    })
}

/// Convert one `name=value`, `name[i]=value`, `name=(…)` or any of their `+=` forms.
///
/// The shape is preserved rather than flattened. Both halves used to be turned into text:
/// `a=(1 2 3)` kept only the first word of `(1 2 3)`, so `echo "$a"` printed the source
/// parentheses, and `a[1]=x` became a variable *literally named* `a[1]` — which looked like it
/// worked only because `${a[1]}` was mangled into the same odd name on the way back out.
fn convert_assignment(assign: &ast::Assignment) -> Result<rush_ast::Assignment> {
    let target = match &assign.name {
        ast::AssignmentName::VariableName(name) => rush_ast::AssignmentTarget::Name(name.clone()),
        ast::AssignmentName::ArrayElementName(name, index) => rush_ast::AssignmentTarget::Element {
            name: name.clone(),
            index: single_word_from_str(index)?,
        },
    };

    let value = match &assign.value {
        ast::AssignmentValue::Scalar(word) => {
            rush_ast::AssignmentValue::Scalar(single_word_from_str(word.as_ref())?)
        }
        ast::AssignmentValue::Array(elements) => {
            let mut converted = Vec::new();
            for (index, value) in elements {
                converted.extend(convert_array_element(index.as_ref(), value)?);
            }
            rush_ast::AssignmentValue::Array(converted)
        }
    };

    Ok(rush_ast::Assignment {
        target,
        value,
        append: assign.append,
    })
}

/// One element of an array literal.
///
/// An unindexed element may still expand to several words — `a=($list)` and `a=(*.c)` both do —
/// so it becomes one [`rush_ast::ArrayElement`] per word rather than being forced into one. An
/// *indexed* element (`[3]=x`) is a single value by construction.
fn convert_array_element(
    index: Option<&brush_parser::ast::Word>,
    value: &ast::Word,
) -> Result<Vec<rush_ast::ArrayElement>> {
    match index {
        Some(index) => Ok(vec![rush_ast::ArrayElement {
            index: Some(single_word_from_str(index.as_ref())?),
            value: single_word_from_str(value.as_ref())?,
        }]),
        None => Ok(convert_braced_words_from_str(value.as_ref())?
            .into_iter()
            .map(|value| rush_ast::ArrayElement { index: None, value })
            .collect()),
    }
}

/// Re-lex `text` as one word, which is what every assignment operand is.
fn single_word_from_str(text: &str) -> Result<rush_ast::Word> {
    Ok(convert_words_from_str(text)?
        .into_iter()
        .next()
        .unwrap_or_else(|| rush_ast::Word::from_literal("")))
}

#[cfg(test)]
mod tests {
    use super::unreachable_construct;

    /// The shapes a `select` loop is actually written in.
    #[test]
    fn select_is_recognised_from_the_source() {
        for script in [
            "select x in a b; do echo $x; done",
            "  select choice in one two\ndo\n  echo $choice\ndone",
            "echo start\nselect\tx in a; do :; done",
        ] {
            assert!(
                unreachable_construct(script).is_some_and(|e| e.to_string().contains("select")),
                "{script:?} should be recognised as select"
            );
        }
    }

    /// A false positive would replace a truthful diagnostic with a misleading one, so the check
    /// has to stay blind to every ordinary use of the word.
    #[test]
    fn ordinary_uses_of_the_word_are_not_recognised() {
        for script in [
            "echo select",
            "x=select",
            "selection=1",
            "select",
            "for w in select; do echo $w; done",
            "grep select file",
            "select 'x' in a",
        ] {
            assert!(
                unreachable_construct(script).is_none(),
                "{script:?} must not be reported as select"
            );
        }
    }
}
