//! Redirection conversion.
//!
//! Covers every redirect brush can produce: files, fd duplication, here-documents,
//! here-strings, and the combined stdout/stderr form.

use super::words::single_word;
use crate::ast as oslo_ast;
use crate::error::Result;
use crate::lexer::parse_heredoc_body;
use brush_parser::ast;

pub(super) fn convert_redirect_list(
    list: &ast::RedirectList,
) -> Result<Vec<oslo_ast::Redirection>> {
    let mut out = Vec::new();
    for r in &list.0 {
        out.extend(convert_redirect(r)?);
    }
    Ok(out)
}

/// Convert one brush redirection.
///
/// Returns a `Vec` because `&>file` expands to two oslo redirections — there is no single node
/// meaning "stdout and stderr".
pub(super) fn convert_redirect(redir: &ast::IoRedirect) -> Result<Vec<oslo_ast::Redirection>> {
    match redir {
        ast::IoRedirect::File(fd, kind, target) => {
            let redirect_kind = match kind {
                ast::IoFileRedirectKind::Read => oslo_ast::RedirectKind::Input,
                ast::IoFileRedirectKind::Write => oslo_ast::RedirectKind::Output,
                ast::IoFileRedirectKind::Append => oslo_ast::RedirectKind::Append,
                ast::IoFileRedirectKind::ReadAndWrite => oslo_ast::RedirectKind::ReadWrite,
                ast::IoFileRedirectKind::Clobber => oslo_ast::RedirectKind::Clobber,
                ast::IoFileRedirectKind::DuplicateInput => oslo_ast::RedirectKind::DupInput,
                ast::IoFileRedirectKind::DuplicateOutput => oslo_ast::RedirectKind::DupOutput,
            };

            let target_word = match target {
                ast::IoFileRedirectTarget::Filename(w) => single_word(w)?,
                ast::IoFileRedirectTarget::Duplicate(w) => single_word(w)?,
                ast::IoFileRedirectTarget::Fd(n) => oslo_ast::Word::from_literal(&n.to_string()),
                // `cmd < <(other)`: the redirect target is the pipe's name, so this is the same
                // word the argument form builds.
                ast::IoFileRedirectTarget::ProcessSubstitution(kind, subshell) => {
                    super::commands::convert_process_substitution(kind, subshell)
                }
            };

            Ok(vec![oslo_ast::Redirection {
                fd: *fd,
                kind: redirect_kind,
                target: target_word,
                heredoc_content: None,
                here_string: false,
            }])
        }

        ast::IoRedirect::HereDocument(fd, doc) => {
            let mut content = doc.doc.to_string();
            // `<<-` strips leading tabs from every line, including the delimiter line. This runs
            // on the raw text, before lexing: a tab is stripped because of where it sits in the
            // line, which is a fact about the source and not about the parts it lexes into.
            if doc.remove_tabs {
                content = content
                    .lines()
                    .map(|l| l.trim_start_matches('\t'))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !content.is_empty() && !content.ends_with('\n') {
                    content.push('\n');
                }
            }

            // A quoted delimiter (`<<'EOF'`) makes the whole body literal, and brush is the one
            // that knows: the quotes are on the delimiter line, which never reaches us.
            //
            // Lexing here rather than at apply time means a body containing an unterminated
            // `${`, `$(` or backtick is a parse error for the whole script, where bash reports
            // it when the redirection runs and carries on. Both refuse — neither shell emits the
            // malformed text — and refusing earlier is the direction this parser errs in
            // everywhere else. A body that is meant to hold such text is a body whose delimiter
            // should be quoted, which is exactly the branch below.
            let body = if doc.requires_expansion {
                parse_heredoc_body(&content)?
            } else {
                oslo_ast::Word::from_literal(&content)
            };

            Ok(vec![oslo_ast::Redirection {
                fd: *fd,
                kind: if doc.remove_tabs {
                    oslo_ast::RedirectKind::HeredocStrip
                } else {
                    oslo_ast::RedirectKind::Heredoc
                },
                target: oslo_ast::Word::from_literal(doc.here_end.as_ref()),
                heredoc_content: Some(body),
                here_string: false,
            }])
        }

        // `<<< word` is a here-document whose body is one ordinary word, expansions and all.
        // `single_word` is what keeps the quoting: a here-string is a single word by
        // construction, so it re-lexes into parts and expansion does the quote removal that a
        // textual strip used to do wrongly (`<<< a"b"c` lost nothing, `<<< "a"x"b"` lost the
        // wrong quotes).
        ast::IoRedirect::HereString(fd, word) => Ok(vec![oslo_ast::Redirection {
            fd: *fd,
            kind: oslo_ast::RedirectKind::Heredoc,
            target: oslo_ast::Word::from_literal(""),
            heredoc_content: Some(single_word(word)?),
            here_string: true,
        }]),

        // `&>file` / `&>>file`: send stdout to the file, then point stderr at stdout.
        ast::IoRedirect::OutputAndError(word, append) => Ok(vec![
            oslo_ast::Redirection {
                fd: Some(1),
                kind: if *append {
                    oslo_ast::RedirectKind::Append
                } else {
                    oslo_ast::RedirectKind::Output
                },
                target: single_word(word)?,
                heredoc_content: None,
                here_string: false,
            },
            oslo_ast::Redirection {
                fd: Some(2),
                kind: oslo_ast::RedirectKind::DupOutput,
                target: oslo_ast::Word::from_literal("1"),
                heredoc_content: None,
                here_string: false,
            },
        ]),
    }
}
