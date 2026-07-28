//! Redirection conversion.
//!
//! Covers every redirect brush can produce: files, fd duplication, here-documents,
//! here-strings, and the combined stdout/stderr form.

use super::words::single_word;
use crate::ast as rush_ast;
use crate::error::{Result, ShellError};
use brush_parser::ast;

pub(super) fn convert_redirect_list(
    list: &ast::RedirectList,
) -> Result<Vec<rush_ast::Redirection>> {
    let mut out = Vec::new();
    for r in &list.0 {
        out.extend(convert_redirect(r)?);
    }
    Ok(out)
}

/// Convert one brush redirection.
///
/// Returns a `Vec` because `&>file` expands to two rush redirections — there is no single node
/// meaning "stdout and stderr".
pub(super) fn convert_redirect(redir: &ast::IoRedirect) -> Result<Vec<rush_ast::Redirection>> {
    match redir {
        ast::IoRedirect::File(fd, kind, target) => {
            let redirect_kind = match kind {
                ast::IoFileRedirectKind::Read => rush_ast::RedirectKind::Input,
                ast::IoFileRedirectKind::Write => rush_ast::RedirectKind::Output,
                ast::IoFileRedirectKind::Append => rush_ast::RedirectKind::Append,
                ast::IoFileRedirectKind::ReadAndWrite => rush_ast::RedirectKind::ReadWrite,
                ast::IoFileRedirectKind::Clobber => rush_ast::RedirectKind::Clobber,
                ast::IoFileRedirectKind::DuplicateInput => rush_ast::RedirectKind::DupInput,
                ast::IoFileRedirectKind::DuplicateOutput => rush_ast::RedirectKind::DupOutput,
            };

            let target_word = match target {
                ast::IoFileRedirectTarget::Filename(w) => single_word(w),
                ast::IoFileRedirectTarget::Duplicate(w) => single_word(w),
                ast::IoFileRedirectTarget::Fd(n) => rush_ast::Word::from_literal(&n.to_string()),
                ast::IoFileRedirectTarget::ProcessSubstitution(..) => {
                    return Err(ShellError::SyntaxError(
                        "process substitution is not supported".to_string(),
                    ));
                }
            };

            Ok(vec![rush_ast::Redirection {
                fd: *fd,
                kind: redirect_kind,
                target: target_word,
                heredoc_content: None,
            }])
        }

        ast::IoRedirect::HereDocument(fd, doc) => {
            let mut content = doc.doc.to_string();
            // `<<-` strips leading tabs from every line, including the delimiter line. The
            // evaluator writes the body verbatim, so strip here.
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

            Ok(vec![rush_ast::Redirection {
                fd: *fd,
                kind: if doc.remove_tabs {
                    rush_ast::RedirectKind::HeredocStrip
                } else {
                    rush_ast::RedirectKind::Heredoc
                },
                target: rush_ast::Word::from_literal(doc.here_end.as_ref()),
                heredoc_content: Some(content),
            }])
        }

        // `<<< word` is a here-document whose body is the single word.
        ast::IoRedirect::HereString(fd, word) => Ok(vec![rush_ast::Redirection {
            fd: *fd,
            kind: rush_ast::RedirectKind::Heredoc,
            target: rush_ast::Word::from_literal(""),
            heredoc_content: Some(format!("{}\n", strip_quotes(word.as_ref()))),
        }]),

        // `&>file` / `&>>file`: send stdout to the file, then point stderr at stdout.
        ast::IoRedirect::OutputAndError(word, append) => Ok(vec![
            rush_ast::Redirection {
                fd: Some(1),
                kind: if *append {
                    rush_ast::RedirectKind::Append
                } else {
                    rush_ast::RedirectKind::Output
                },
                target: single_word(word),
                heredoc_content: None,
            },
            rush_ast::Redirection {
                fd: Some(2),
                kind: rush_ast::RedirectKind::DupOutput,
                target: rush_ast::Word::from_literal("1"),
                heredoc_content: None,
            },
        ]),
    }
}

/// Strip one layer of matching surrounding quotes.
///
/// Here-string bodies are written to the pipe verbatim, so quotes that were syntax rather than
/// content would otherwise show up in the output.
fn strip_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        return s[1..s.len() - 1].to_string();
    }
    s.to_string()
}
