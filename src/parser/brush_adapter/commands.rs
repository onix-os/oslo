//! Command and compound-command conversion.
//!
//! Turns brush's `Command` and `CompoundCommand` nodes into rush's equivalents: simple
//! commands with their assignments and redirections, `if`/`while`/`for`/`case` bodies, and
//! function definitions.

use super::convert_compound_list;
use super::extended_test::convert_extended_test;
use super::redirects::{convert_redirect, convert_redirect_list};
use super::words::{convert_word, convert_words_from_str, single_command_list};
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
                        converted.extend(convert_word(w));
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
        // These parse but have no representation in rush's AST. Naming the construct matters:
        // this diagnostic is now the whole of the user's feedback, where it used to be swallowed
        // by a fallback parser that reinterpreted the entire program.
        ast::CompoundCommand::Arithmetic(_) => {
            return Err(unsupported("((...)) arithmetic command"));
        }
        ast::CompoundCommand::ArithmeticForClause(_) => {
            return Err(unsupported("for ((...)) arithmetic loop"));
        }
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
    let word = convert_word(&case_clause.value)
        .into_iter()
        .next()
        .unwrap_or_else(|| rush_ast::Word::from_literal(""));

    let mut items = Vec::new();
    for case in &case_clause.cases {
        let mut patterns = Vec::new();
        for p in &case.patterns {
            patterns.extend(convert_word(p));
        }

        let body = match &case.cmd {
            Some(list) => convert_compound_list(list)?,
            None => rush_ast::CommandList::default(),
        };

        items.push(rush_ast::CaseItem { patterns, body });
    }

    Ok(rush_ast::CompoundCommand::Case { word, items })
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
                    assignments.push(convert_assignment(assign));
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
                    redirections.extend(convert_redirect(redir)?);
                }
                // Process substitution is not representable in rush's AST.
                ast::CommandPrefixOrSuffixItem::Word(_)
                | ast::CommandPrefixOrSuffixItem::ProcessSubstitution(..) => {}
            }
        }
    }

    if let Some(cmd_word) = &simple.word_or_name {
        words.extend(convert_word(cmd_word));
    }

    if let Some(suffix) = &simple.suffix {
        for item in &suffix.0 {
            match item {
                ast::CommandPrefixOrSuffixItem::Word(w) => {
                    words.extend(convert_word(w));
                }
                ast::CommandPrefixOrSuffixItem::AssignmentWord(_, raw) => {
                    // Only a *prefix* `name=value` is an assignment; after the command name it
                    // is an ordinary argument (`alias g='echo hi'`, `env FOO=bar`).
                    //
                    // Use brush's raw source word rather than re-joining name and value: the
                    // rejoined text would be a bare literal, so its quotes would survive
                    // expansion and the value would then be field-split on its own spaces.
                    words.extend(convert_word(raw));
                }
                ast::CommandPrefixOrSuffixItem::IoRedirect(redir) => {
                    redirections.extend(convert_redirect(redir)?);
                }
                ast::CommandPrefixOrSuffixItem::ProcessSubstitution(..) => {}
            }
        }
    }

    Ok(rush_ast::SimpleCommand {
        assignments,
        words,
        redirections,
    })
}

fn convert_assignment(assign: &ast::Assignment) -> rush_ast::Assignment {
    let value = convert_words_from_str(&assign.value.to_string())
        .into_iter()
        .next()
        .unwrap_or_else(|| rush_ast::Word::from_literal(""));
    rush_ast::Assignment {
        name: assign.name.to_string(),
        value,
    }
}
