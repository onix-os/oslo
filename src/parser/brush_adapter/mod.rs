//! Bridge from [`brush_parser`]'s AST to oslo's own [`crate::ast`].
//!
//! brush is a spec-compliant bash parser; oslo's evaluator works on its own simpler AST. This
//! module is the translation layer, and it is the *only* path from source text to a runnable
//! program: `main`, `eval`, `source`, command substitution and the Lua `oslo.proc.exec` binding all
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
//! - Process substitution (`<(cmd)`, `>(cmd)`) is not representable, and is refused by name
//!   wherever it appears — as a redirect target and as a word. It used to be *dropped* from a
//!   word position, so `cat <(echo hi)` ran `cat` with no arguments and exited 0.
//! - `coproc` and `select` are refused by name rather than approximated; `select` is not in
//!   brush 0.4's grammar at all, so it is recognised from the source text (`unreachable_construct`
//!   in `commands.rs`) to keep the diagnostic from reading like a typo near `in`.
//! - `for ((;;))` written without spaces is a parse error: brush 0.4's tokenizer reads the `;;`
//!   as the `case` terminator while its grammar wants two `;` operators. `for (( ; ; ))` works.
//! - `time -p` is carried as plain `time`: [`crate::ast::Pipeline`] records *that* a pipeline is
//!   timed, not which of bash's two report formats was asked for, so `-p` gets the default
//!   three-line form instead of POSIX's single-space one. The measurement itself is identical.
//! - `[[ ... ]]` becomes calls to the `[[` builtin, with `&&`, `||` and `!` expressed using the
//!   shell's own and-or lists and pipeline negation. `=~` is rejected: there is no regex engine,
//!   and approximating it would be worse than refusing.

mod commands;
mod extended_test;
mod redirects;
mod words;

#[cfg(test)]
mod tests;

use crate::ast as oslo_ast;
use crate::error::{Result, ShellError};
use brush_parser::{ParserOptions, ast};
use commands::convert_command;
use std::io::Cursor;

pub fn parse_bash_script(script: &str) -> Result<oslo_ast::CommandList> {
    // Before brush sees the text, not after: brush is recursive descent, so absurdly nested input
    // overflows the stack inside `parse_program` and aborts the process before any error of ours
    // could be produced. See [`crate::nesting`].
    crate::nesting::check_nesting(script)?;

    let script_buf = format!("{}\n", script);
    let cursor = Cursor::new(script_buf.as_bytes());
    let parser_opts = ParserOptions::default();
    let mut parser = brush_parser::Parser::new(cursor, &parser_opts);

    match parser.parse_program() {
        Ok(prog) => convert_program(&prog),
        // R8.6: a construct brush's grammar does not have at all fails at whatever token follows
        // it, so `select x in a b` reads as a typo near `in`. Name the construct when we can
        // recognise it; otherwise brush's own message already carries the position, and wrapping
        // it in more prose would only bury the line and column the user needs.
        Err(e) => Err(commands::unreachable_construct(script)
            .unwrap_or_else(|| ShellError::SyntaxError(e.to_string()))),
    }
}

fn convert_program(prog: &ast::Program) -> Result<oslo_ast::CommandList> {
    let mut items = Vec::new();

    for compound_list in &prog.complete_commands {
        for list_item in &compound_list.0 {
            items.push(convert_list_item(list_item)?);
        }
    }

    Ok(oslo_ast::CommandList { items })
}

pub(super) fn convert_compound_list(list: &ast::CompoundList) -> Result<oslo_ast::CommandList> {
    let mut items = Vec::new();
    for list_item in &list.0 {
        items.push(convert_list_item(list_item)?);
    }
    Ok(oslo_ast::CommandList { items })
}

/// Convert one list item, preserving its separator.
///
/// The separator is what distinguishes `sleep 10 &` from `sleep 10;` — dropping it silently
/// turns every background job into a foreground one.
fn convert_list_item(item: &ast::CompoundListItem) -> Result<oslo_ast::ListItem> {
    // brush records where each node came from; this is the only place that reads it, and it is
    // what lets `$LINENO` name a real line instead of being empty.
    let line = {
        use brush_parser::ast::SourceLocation as _;
        item.0
            .location()
            .map(|span| span.start.line as u32)
            .unwrap_or(0)
    };
    let and_or = convert_and_or(&item.0)?;
    let op = match item.1 {
        ast::SeparatorOperator::Async => oslo_ast::ListOp::Background,
        ast::SeparatorOperator::Sequence => oslo_ast::ListOp::Sequential,
    };
    Ok(oslo_ast::ListItem { and_or, op, line })
}

fn convert_and_or(and_or: &ast::AndOrList) -> Result<oslo_ast::AndOrList> {
    let first = convert_pipeline(&and_or.first)?;
    let mut rest = Vec::new();

    for item in &and_or.additional {
        let (op, pipeline) = match item {
            ast::AndOr::And(p) => (oslo_ast::AndOrOp::And, convert_pipeline(p)?),
            ast::AndOr::Or(p) => (oslo_ast::AndOrOp::Or, convert_pipeline(p)?),
        };
        rest.push((op, pipeline));
    }

    Ok(oslo_ast::AndOrList { first, rest })
}

fn convert_pipeline(pipe: &ast::Pipeline) -> Result<oslo_ast::Pipeline> {
    let mut commands = Vec::new();
    for cmd in &pipe.seq {
        commands.push(convert_command(cmd)?);
    }
    Ok(oslo_ast::Pipeline {
        negated: pipe.bang,
        // R8.7: brush records the keyword and oslo used to read only `bang` and `seq`, so `time`
        // was accepted and then silently discarded. `-p` is flattened into the same flag; see the
        // known gap above.
        timed: pipe.timed.is_some(),
        commands,
    })
}
