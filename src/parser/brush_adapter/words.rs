//! Word conversion and small AST constructors.
//!
//! brush hands back a word's raw source text; rush's evaluator needs it decomposed into
//! [`crate::ast::WordPart`]s so expansion can run, which means re-lexing it here.

use crate::ast as rush_ast;
use brush_parser::ast;

/// Wrap a single command as a `CommandList`.
pub(super) fn single_command_list(cmd: rush_ast::Command) -> rush_ast::CommandList {
    rush_ast::CommandList {
        items: vec![rush_ast::ListItem {
            and_or: rush_ast::AndOrList {
                first: rush_ast::Pipeline {
                    negated: false,
                    timed: false,
                    commands: vec![cmd],
                },
                rest: Vec::new(),
            },
            op: rush_ast::ListOp::Sequential,
        }],
    }
}

/// Wrap an and-or list as a single grouped command.
pub(super) fn single_command(and_or: rush_ast::AndOrList) -> rush_ast::Command {
    rush_ast::Command::Compound {
        kind: rush_ast::CompoundCommand::Group(rush_ast::CommandList {
            items: vec![rush_ast::ListItem {
                and_or,
                op: rush_ast::ListOp::Sequential,
            }],
        }),
        redirections: Vec::new(),
    }
}

/// Convert one brush word, which is already a single shell word.
pub(super) fn single_word(w: &ast::Word) -> rush_ast::Word {
    convert_word(w)
        .into_iter()
        .next()
        .unwrap_or_else(|| rush_ast::Word::from_literal(""))
}

pub(super) fn convert_word(w: &ast::Word) -> Vec<rush_ast::Word> {
    convert_words_from_str(w.as_ref())
}

/// Re-lex brush's raw word text with rush's own lexer.
///
/// brush hands back the word's source text (`"$HOME/x"`, `'a b'`); rush's evaluator needs it
/// decomposed into [`crate::ast::WordPart`]s so expansion can run. Re-lexing is the bridge.
pub(super) fn convert_words_from_str(word_str: &str) -> Vec<rush_ast::Word> {
    let trimmed = word_str.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let mut lexer = crate::lexer::Lexer::new(trimmed);
    let mut words = Vec::new();

    loop {
        match lexer.next() {
            Ok(crate::lexer::Token::Word(w)) => words.push(w),
            Ok(crate::lexer::Token::Eof) => break,
            // Anything else means the text is not a plain word after all (an operator slipped
            // through). Fall back to treating the whole thing as a literal.
            Ok(_) | Err(_) => return vec![rush_ast::Word::from_literal(trimmed)],
        }
    }

    if words.is_empty() {
        vec![rush_ast::Word::from_literal(trimmed)]
    } else {
        words
    }
}
