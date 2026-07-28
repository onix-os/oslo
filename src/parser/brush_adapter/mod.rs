//! Bridge from [`brush_parser`]'s AST to rush's own [`crate::ast`].
//!
//! brush is a spec-compliant bash parser; rush's evaluator works on its own simpler AST. This
//! module is the translation layer, and it is the *only* path from source text to a runnable
//! program: `main`, `eval`, `source`, command substitution and the Lua `rush.exec` binding all
//! come through here, and an error raised here is what the user sees.
//!
//! That makes fidelity here load-bearing: anything this module drops is silently unobservable at
//! runtime. Anything it cannot represent must therefore be *rejected by name* (see the
//! unsupported-construct arm in `commands.rs`), never approximated.
//!
//! # Known gaps
//!
//! - Here-document bodies are carried through verbatim; parameter expansion inside an unquoted
//!   heredoc is not applied (`brush`'s `requires_expansion` flag is recorded but unused).
//! - `case` fallthrough (`;&`, `;;&`) is parsed but executed as `;;`, because
//!   [`crate::ast::CaseItem`] has nowhere to record the post-action.
//! - Process substitution (`<(cmd)`, `>(cmd)`) is not representable and is dropped.
//! - `[[ ... ]]` becomes calls to the `[[` builtin, with `&&`, `||` and `!` expressed using the
//!   shell's own and-or lists and pipeline negation. `=~` is rejected: there is no regex engine,
//!   and approximating it would be worse than refusing.

mod commands;
mod extended_test;
mod redirects;
mod words;

#[cfg(test)]
mod tests;

use crate::ast as rush_ast;
use crate::error::{Result, ShellError};
use brush_parser::{ParserOptions, ast};
use commands::convert_command;
use std::io::Cursor;

pub fn parse_bash_script(script: &str) -> Result<rush_ast::CommandList> {
    // Before brush sees the text, not after: brush is recursive descent, so absurdly nested input
    // overflows the stack inside `parse_program` and aborts the process before any error of ours
    // could be produced. See [`crate::parser::nesting`].
    crate::parser::nesting::check_nesting(script)?;

    let script_buf = format!("{}\n", script);
    let cursor = Cursor::new(script_buf.as_bytes());
    let parser_opts = ParserOptions::default();
    let mut parser = brush_parser::Parser::new(cursor, &parser_opts);

    match parser.parse_program() {
        Ok(prog) => convert_program(&prog),
        // brush's own message already names the position; wrapping it in more prose would only
        // bury the line and column the user needs.
        Err(e) => Err(ShellError::SyntaxError(e.to_string())),
    }
}

fn convert_program(prog: &ast::Program) -> Result<rush_ast::CommandList> {
    let mut items = Vec::new();

    for compound_list in &prog.complete_commands {
        for list_item in &compound_list.0 {
            items.push(convert_list_item(list_item)?);
        }
    }

    Ok(rush_ast::CommandList { items })
}

pub(super) fn convert_compound_list(list: &ast::CompoundList) -> Result<rush_ast::CommandList> {
    let mut items = Vec::new();
    for list_item in &list.0 {
        items.push(convert_list_item(list_item)?);
    }
    Ok(rush_ast::CommandList { items })
}

/// Convert one list item, preserving its separator.
///
/// The separator is what distinguishes `sleep 10 &` from `sleep 10;` — dropping it silently
/// turns every background job into a foreground one.
fn convert_list_item(item: &ast::CompoundListItem) -> Result<rush_ast::ListItem> {
    let and_or = convert_and_or(&item.0)?;
    let op = match item.1 {
        ast::SeparatorOperator::Async => rush_ast::ListOp::Background,
        ast::SeparatorOperator::Sequence => rush_ast::ListOp::Sequential,
    };
    Ok(rush_ast::ListItem { and_or, op })
}

fn convert_and_or(and_or: &ast::AndOrList) -> Result<rush_ast::AndOrList> {
    let first = convert_pipeline(&and_or.first)?;
    let mut rest = Vec::new();

    for item in &and_or.additional {
        let (op, pipeline) = match item {
            ast::AndOr::And(p) => (rush_ast::AndOrOp::And, convert_pipeline(p)?),
            ast::AndOr::Or(p) => (rush_ast::AndOrOp::Or, convert_pipeline(p)?),
        };
        rest.push((op, pipeline));
    }

    Ok(rush_ast::AndOrList { first, rest })
}

fn convert_pipeline(pipe: &ast::Pipeline) -> Result<rush_ast::Pipeline> {
    let mut commands = Vec::new();
    for cmd in &pipe.seq {
        commands.push(convert_command(cmd)?);
    }
    Ok(rush_ast::Pipeline {
        negated: pipe.bang,
        commands,
    })
}
