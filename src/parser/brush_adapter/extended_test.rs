//! `[[ ... ]]` conversion.
//!
//! oslo has no dedicated node for extended tests, so the expression tree is lowered onto
//! constructs it already evaluates: `&&`/`||` become an and-or list, `!` becomes a negated
//! pipeline, and each leaf predicate becomes a call to the `[[` builtin.

use super::words::{single_command, single_word};
use crate::ast as oslo_ast;
use crate::error::Result;
use brush_parser::ast;

pub(super) fn convert_extended_test(expr: &ast::ExtendedTestExpr) -> Result<oslo_ast::Command> {
    Ok(single_command(extended_test_to_and_or(expr)?))
}

fn extended_test_to_and_or(expr: &ast::ExtendedTestExpr) -> Result<oslo_ast::AndOrList> {
    match expr {
        ast::ExtendedTestExpr::Parenthesized(inner) => extended_test_to_and_or(inner),
        ast::ExtendedTestExpr::And(l, r) => {
            let mut left = extended_test_to_and_or(l)?;
            let right = extended_test_to_and_or(r)?;
            left.rest.push((oslo_ast::AndOrOp::And, right.first));
            left.rest.extend(right.rest);
            Ok(left)
        }
        ast::ExtendedTestExpr::Or(l, r) => {
            let mut left = extended_test_to_and_or(l)?;
            let right = extended_test_to_and_or(r)?;
            left.rest.push((oslo_ast::AndOrOp::Or, right.first));
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
                let grouped = oslo_ast::Command::Compound {
                    kind: oslo_ast::CompoundCommand::Group(oslo_ast::CommandList {
                        items: vec![oslo_ast::ListItem {
                            and_or: list,
                            op: oslo_ast::ListOp::Sequential,
                            line: 0,
                        }],
                    }),
                    redirections: Vec::new(),
                };
                Ok(oslo_ast::AndOrList {
                    first: oslo_ast::Pipeline {
                        negated: true,
                        timed: false,
                        commands: vec![grouped],
                    },
                    rest: Vec::new(),
                })
            }
        }
        ast::ExtendedTestExpr::UnaryTest(pred, word) => {
            let op = unary_predicate_op(pred);
            Ok(bracket_and_or(
                vec![oslo_ast::Word::from_literal(op), operand(word)?],
                false,
            ))
        }
        ast::ExtendedTestExpr::BinaryTest(pred, left, right) => {
            let (op, negate) = binary_predicate_op(pred, right);
            Ok(bracket_and_or(
                vec![
                    operand(left)?,
                    oslo_ast::Word::from_literal(op),
                    operand(right)?,
                ],
                negate,
            ))
        }
    }
}

/// One operand of a predicate, in a form that cannot become anything other than one word.
///
/// This is the difference between `[[ ]]` and `[ ]` that the lowering would otherwise lose. `[[`
/// is a *syntactic* construct: its operands are not field-split and not pathname-expanded, so
/// `[[ $x == "a b" ]]` is a comparison and `[[ -n $x ]]` with an empty `x` is still a test on one
/// (empty) operand. Lowered to an ordinary command, they were expanded like ordinary arguments —
/// so a value with a space became two operands (`too many arguments`), a value containing `*` was
/// globbed against the working directory, and an empty value vanished, shifting the operator into
/// the operand slot.
///
/// Wrapping the whole word in double quotes says exactly that, and reuses the expansion rules
/// already written rather than adding a second set. It does not make the `==` right-hand side
/// literal: pattern-versus-text is decided from the *source* quoting by [`binary_predicate_op`],
/// before this runs, and is carried in the operator word.
fn operand(word: &ast::Word) -> Result<oslo_ast::Word> {
    let inner = single_word(word)?;
    Ok(oslo_ast::Word {
        parts: vec![oslo_ast::WordPart::DoubleQuoted(inner.parts)],
    })
}

/// Build a `[[ <args> ]]` invocation as a one-pipeline and-or list.
///
/// `negate` sets the pipeline's `!`, which is how the negative predicates (`!=`) are expressed —
/// so the builtin only has to implement the positive comparisons.
fn bracket_and_or(args: Vec<oslo_ast::Word>, negate: bool) -> oslo_ast::AndOrList {
    let mut words = vec![oslo_ast::Word::from_literal("[[")];
    words.extend(args);
    words.push(oslo_ast::Word::from_literal("]]"));

    oslo_ast::AndOrList {
        first: oslo_ast::Pipeline {
            negated: negate,
            timed: false,
            commands: vec![oslo_ast::Command::Simple(oslo_ast::SimpleCommand {
                assignments: Vec::new(),
                words,
                redirections: Vec::new(),
            })],
        },
        rest: Vec::new(),
    }
}

/// Map a `[[ -x ... ]]` predicate onto the flag understood by the `[[` builtin.
///
/// Total, and deliberately without a catch-all: every predicate brush can parse has a flag, so a
/// new one appearing in a future brush-parser release is a compile error here rather than a
/// runtime `unsupported test predicate` that only the affected script would discover.
fn unary_predicate_op(pred: &ast::UnaryPredicate) -> &'static str {
    use ast::UnaryPredicate as P;
    match pred {
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
        // The rest of bash's `test_unop` set. The builtin's table already implements every one of
        // these; only this mapping was missing, so `[[ -o errexit ]]` was a *syntax error* while
        // `[ -o errexit ]` answered correctly — the same predicate, two verdicts.
        P::FileExistsAndIsSetuid => "-u",
        P::FileExistsAndIsSetgid => "-g",
        P::FileExistsAndHasStickyBit => "-k",
        P::FileExistsAndOwnedByEffectiveUserId => "-O",
        P::FileExistsAndOwnedByEffectiveGroupId => "-G",
        P::FileExistsAndModifiedSinceLastRead => "-N",
        P::FdIsOpenTerminal => "-t",
        P::ShellOptionEnabled => "-o",
        P::ShellVariableIsSetAndNameRef => "-R",
    }
}

/// Map a binary predicate to `(operator, negate)`.
///
/// In `[[ ]]`, `=` and `==` are *pattern* matches, not string equality — `[[ abc == a* ]]` is
/// true. Quoting the right-hand side turns off pattern matching, and brush preserves the quotes
/// in the word's raw text, so that is what decides between the two operators here.
fn binary_predicate_op(pred: &ast::BinaryPredicate, rhs: &ast::Word) -> (&'static str, bool) {
    use ast::BinaryPredicate as P;

    let pattern_op = if is_quoted(rhs.as_ref()) { "=" } else { "==" };

    match pred {
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
        // `=~`. Quoting the right operand makes it literal text, exactly as it does for `==`, and
        // brush has already split the two cases into separate predicates — `StringContainsSubstring`
        // is the one it produces when the operand's source text starts with a quote. The builtin
        // spells the literal form `=~lit`; see `env::builtins::conditionals::matching`.
        P::StringMatchesRegex => ("=~", false),
        P::StringContainsSubstring => ("=~lit", false),
    }
}

/// Whether a word's raw source text is fully wrapped in quotes.
fn is_quoted(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
}
