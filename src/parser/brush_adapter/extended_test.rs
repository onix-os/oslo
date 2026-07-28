//! `[[ ... ]]` conversion.
//!
//! rush has no dedicated node for extended tests, so the expression tree is lowered onto
//! constructs it already evaluates: `&&`/`||` become an and-or list, `!` becomes a negated
//! pipeline, and each leaf predicate becomes a call to the `[[` builtin.

use super::words::{single_command, single_word};
use crate::ast as rush_ast;
use crate::error::{Result, ShellError};
use brush_parser::ast;

pub(super) fn convert_extended_test(expr: &ast::ExtendedTestExpr) -> Result<rush_ast::Command> {
    Ok(single_command(extended_test_to_and_or(expr)?))
}

fn extended_test_to_and_or(expr: &ast::ExtendedTestExpr) -> Result<rush_ast::AndOrList> {
    match expr {
        ast::ExtendedTestExpr::Parenthesized(inner) => extended_test_to_and_or(inner),
        ast::ExtendedTestExpr::And(l, r) => {
            let mut left = extended_test_to_and_or(l)?;
            let right = extended_test_to_and_or(r)?;
            left.rest.push((rush_ast::AndOrOp::And, right.first));
            left.rest.extend(right.rest);
            Ok(left)
        }
        ast::ExtendedTestExpr::Or(l, r) => {
            let mut left = extended_test_to_and_or(l)?;
            let right = extended_test_to_and_or(r)?;
            left.rest.push((rush_ast::AndOrOp::Or, right.first));
            left.rest.extend(right.rest);
            Ok(left)
        }
        ast::ExtendedTestExpr::Not(inner) => {
            let mut list = extended_test_to_and_or(inner)?;
            // Negation applies to the leading pipeline; a compound expression is grouped first
            // so `! (a && b)` does not silently become `(! a) && b`.
            if list.rest.is_empty() {
                list.first.negated = !list.first.negated;
                Ok(list)
            } else {
                let grouped = rush_ast::Command::Compound {
                    kind: rush_ast::CompoundCommand::Group(rush_ast::CommandList {
                        items: vec![rush_ast::ListItem {
                            and_or: list,
                            op: rush_ast::ListOp::Sequential,
                        }],
                    }),
                    redirections: Vec::new(),
                };
                Ok(rush_ast::AndOrList {
                    first: rush_ast::Pipeline {
                        negated: true,
                        commands: vec![grouped],
                    },
                    rest: Vec::new(),
                })
            }
        }
        ast::ExtendedTestExpr::UnaryTest(pred, word) => {
            let op = unary_predicate_op(pred)?;
            Ok(bracket_and_or(
                vec![rush_ast::Word::from_literal(op), single_word(word)],
                false,
            ))
        }
        ast::ExtendedTestExpr::BinaryTest(pred, left, right) => {
            let (op, negate) = binary_predicate_op(pred, right)?;
            Ok(bracket_and_or(
                vec![
                    single_word(left),
                    rush_ast::Word::from_literal(op),
                    single_word(right),
                ],
                negate,
            ))
        }
    }
}

/// Build a `[[ <args> ]]` invocation as a one-pipeline and-or list.
///
/// `negate` sets the pipeline's `!`, which is how the negative predicates (`!=`) are expressed —
/// so the builtin only has to implement the positive comparisons.
fn bracket_and_or(args: Vec<rush_ast::Word>, negate: bool) -> rush_ast::AndOrList {
    let mut words = vec![rush_ast::Word::from_literal("[[")];
    words.extend(args);
    words.push(rush_ast::Word::from_literal("]]"));

    rush_ast::AndOrList {
        first: rush_ast::Pipeline {
            negated: negate,
            commands: vec![rush_ast::Command::Simple(rush_ast::SimpleCommand {
                assignments: Vec::new(),
                words,
                redirections: Vec::new(),
            })],
        },
        rest: Vec::new(),
    }
}

/// Map a `[[ -x ... ]]` predicate onto the flag understood by the `[[` builtin.
fn unary_predicate_op(pred: &ast::UnaryPredicate) -> Result<&'static str> {
    use ast::UnaryPredicate as P;
    Ok(match pred {
        P::FileExists => "-e",
        P::FileExistsAndIsRegularFile => "-f",
        P::FileExistsAndIsDir => "-d",
        P::FileExistsAndIsReadable => "-r",
        P::FileExistsAndIsWritable => "-w",
        P::FileExistsAndIsExecutable => "-x",
        P::FileExistsAndIsSymlink => "-L",
        P::FileExistsAndIsNotZeroLength => "-s",
        P::FileExistsAndIsFifo => "-p",
        P::FileExistsAndIsSocket => "-S",
        P::FileExistsAndIsBlockSpecialFile => "-b",
        P::FileExistsAndIsCharSpecialFile => "-c",
        P::StringHasZeroLength => "-z",
        P::StringHasNonZeroLength => "-n",
        P::ShellVariableIsSetAndAssigned => "-v",
        other => {
            return Err(ShellError::SyntaxError(format!(
                "unsupported test predicate: {}",
                other
            )));
        }
    })
}

/// Map a binary predicate to `(operator, negate)`.
///
/// In `[[ ]]`, `=` and `==` are *pattern* matches, not string equality — `[[ abc == a* ]]` is
/// true. Quoting the right-hand side turns off pattern matching, and brush preserves the quotes
/// in the word's raw text, so that is what decides between the two operators here.
fn binary_predicate_op(
    pred: &ast::BinaryPredicate,
    rhs: &ast::Word,
) -> Result<(&'static str, bool)> {
    use ast::BinaryPredicate as P;

    let pattern_op = if is_quoted(rhs.as_ref()) { "=" } else { "==" };

    Ok(match pred {
        P::StringExactlyMatchesPattern => (pattern_op, false),
        P::StringDoesNotExactlyMatchPattern => (pattern_op, true),
        P::StringExactlyMatchesString => ("=", false),
        P::StringDoesNotExactlyMatchString => ("=", true),
        P::LeftSortsBeforeRight => ("<", false),
        P::LeftSortsAfterRight => (">", false),
        P::ArithmeticEqualTo => ("-eq", false),
        P::ArithmeticNotEqualTo => ("-ne", false),
        P::ArithmeticLessThan => ("-lt", false),
        P::ArithmeticLessThanOrEqualTo => ("-le", false),
        P::ArithmeticGreaterThan => ("-gt", false),
        P::ArithmeticGreaterThanOrEqualTo => ("-ge", false),
        P::LeftFileIsNewerOrExistsWhenRightDoesNot => ("-nt", false),
        P::LeftFileIsOlderOrDoesNotExistWhenRightDoes => ("-ot", false),
        P::FilesReferToSameDeviceAndInodeNumbers => ("-ef", false),
        // Regex matching would need a regex engine; refuse rather than approximate it.
        other => {
            return Err(ShellError::SyntaxError(format!(
                "unsupported test predicate: {}",
                other
            )));
        }
    })
}

/// Whether a word's raw source text is fully wrapped in quotes.
fn is_quoted(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
}
