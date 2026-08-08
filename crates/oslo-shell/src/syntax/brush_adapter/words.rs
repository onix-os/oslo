//! Word conversion and small AST constructors.
//!
//! brush hands back a word's raw source text; oslo's evaluator needs it decomposed into
//! [`oslo_base::ast::WordPart`]s so expansion can run, which means re-lexing it here.

use brush_parser::ast;
use oslo_base::ast as oslo_ast;
use oslo_base::error::Result;

/// Wrap a single command as a `CommandList`.
pub(super) fn single_command_list(cmd: oslo_ast::Command) -> oslo_ast::CommandList {
    oslo_ast::CommandList {
        items: vec![oslo_ast::ListItem {
            and_or: oslo_ast::AndOrList {
                first: oslo_ast::Pipeline {
                    negated: false,
                    timed: false,
                    commands: vec![cmd],
                },
                rest: Vec::new(),
            },
            op: oslo_ast::ListOp::Sequential,
            line: 0,
        }],
    }
}

/// Wrap an and-or list as a single grouped command.
pub(super) fn single_command(and_or: oslo_ast::AndOrList) -> oslo_ast::Command {
    oslo_ast::Command::Compound {
        kind: oslo_ast::CompoundCommand::Group(oslo_ast::CommandList {
            items: vec![oslo_ast::ListItem {
                and_or,
                op: oslo_ast::ListOp::Sequential,
                line: 0,
            }],
        }),
        redirections: Vec::new(),
    }
}

/// Convert one brush word, which is already a single shell word.
pub(super) fn single_word(w: &ast::Word) -> Result<oslo_ast::Word> {
    Ok(convert_word(w)?
        .into_iter()
        .next()
        .unwrap_or_else(|| oslo_ast::Word::from_literal("")))
}

pub(super) fn convert_word(w: &ast::Word) -> Result<Vec<oslo_ast::Word>> {
    convert_words_from_str(w.as_ref())
}

/// Convert one brush word that stands in a *list of words*, where brace expansion applies.
///
/// Which positions those are is not a detail — it is the difference between `mkdir -p a/{b,c}`
/// making two directories and `w={a,b}` storing the two characters `{a`. bash brace-expands the
/// words of a simple command, the list of a `for`, and the elements of an array literal, and
/// nothing else: not a `case` pattern, not the right-hand side of an assignment, not a subscript.
/// So this is the *opt-in* converter and [`convert_word`] stays the default, which means a caller
/// that should have used this one loses brace expansion — visible immediately, and covered by the
/// corpus — rather than a caller that should not have used it silently rewriting a pattern.
pub(super) fn convert_braced_word(w: &ast::Word) -> Result<Vec<oslo_ast::Word>> {
    convert_braced_words_from_str(w.as_ref())
}

/// [`convert_braced_word`] for a word oslo only has as text, such as an array literal element.
pub(super) fn convert_braced_words_from_str(word_str: &str) -> Result<Vec<oslo_ast::Word>> {
    let trimmed = word_str.trim();
    let mut words = Vec::new();
    for text in crate::expand::brace::expand_braces_text(trimmed) {
        words.extend(convert_words_from_str(&text)?);
    }
    Ok(words)
}

/// Re-lex brush's raw word text with oslo's own lexer.
///
/// brush hands back the word's source text (`"$HOME/x"`, `'a b'`); oslo's evaluator needs it
/// decomposed into [`oslo_base::ast::WordPart`]s so expansion can run. Re-lexing is the bridge.
///
/// The loop is bounded by an explicit progress check rather than by trusting the lexer. It used
/// to trust it, and a character the token scanner skipped but the word scanner refused (a pasted
/// no-break space was enough) made `next()` hand back empty words forever while this `Vec` grew
/// until the allocator aborted the process — at *parse* time, so no script could guard against
/// it. The disagreement itself is fixed in [`crate::lexer::scanner::is_blank`]; this is the check
/// that keeps the next one from being a hang. It must stay an error and never a silent
/// truncation: a word this function cannot lex is a word the shell must not go on to run.
pub(super) fn convert_words_from_str(word_str: &str) -> Result<Vec<oslo_ast::Word>> {
    let trimmed = word_str.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut lexer = crate::lexer::Lexer::new(trimmed);
    let mut words = Vec::new();

    loop {
        let before = lexer.offset();
        match lexer.next() {
            Ok(crate::lexer::Token::Word(w)) => words.push(w),
            Ok(crate::lexer::Token::Eof) => break,
            // Anything else means the text is not a plain word after all (an operator slipped
            // through). Fall back to treating the whole thing as a literal.
            Ok(_) | Err(_) => return Ok(vec![oslo_ast::Word::from_literal(trimmed)]),
        }
        if lexer.offset() == before {
            return Err(oslo_base::error::ShellError::SyntaxError(format!(
                "cannot lex the word {trimmed:?}: the lexer returned a token without consuming \
                 any input"
            )));
        }
    }

    if words.is_empty() {
        Ok(vec![oslo_ast::Word::from_literal(trimmed)])
    } else {
        Ok(words)
    }
}
