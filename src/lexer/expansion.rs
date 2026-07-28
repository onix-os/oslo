//! Scanning `$` expansions.
//!
//! Covers `$var`, `${...}` in all its parameter-expansion forms, `$(...)` and backtick command
//! substitution, and `$((...))` arithmetic.

use super::scanner::{Lexer, is_special_param, is_var_name_char};
use crate::ast::{ParamExpansion, WordPart};
use crate::error::Result;

impl Lexer<'_> {
    pub(super) fn scan_dollar_expansion(&mut self) -> Result<WordPart> {
        match self.current_char() {
            Some('{') => {
                self.advance();
                self.scan_braced_param()
            }
            Some('(') => {
                self.advance();
                if self.current_char() == Some('(') {
                    self.advance();
                    // $(( arithmetic ))
                    let mut expr = String::new();
                    let mut depth = 2;
                    while let Some(c) = self.current_char() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                self.advance();
                                break;
                            } else if depth == 1 && self.peek_char() == Some(')') {
                                self.advance();
                                self.advance();
                                break;
                            }
                        }
                        expr.push(c);
                        self.advance();
                    }
                    Ok(WordPart::Arithmetic(expr))
                } else {
                    // $( command sub )
                    let mut cmd = String::new();
                    let mut depth = 1;
                    while let Some(c) = self.current_char() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                            if depth == 0 {
                                self.advance();
                                break;
                            }
                        }
                        cmd.push(c);
                        self.advance();
                    }
                    Ok(WordPart::CommandSubstitution(cmd))
                }
            }
            Some('`') => {
                self.advance();
                let mut cmd = String::new();
                while let Some(c) = self.current_char() {
                    if c == '`' {
                        self.advance();
                        break;
                    } else if c == '\\'
                        && matches!(self.peek_char(), Some('`') | Some('\\') | Some('$'))
                    {
                        self.advance();
                        cmd.push(self.current_char().unwrap());
                        self.advance();
                    } else {
                        cmd.push(c);
                        self.advance();
                    }
                }
                Ok(WordPart::CommandSubstitution(cmd))
            }
            Some(ch) if is_var_name_char(ch) || is_special_param(ch) => {
                let mut name = String::new();
                if is_special_param(ch) {
                    name.push(ch);
                    self.advance();
                } else {
                    while let Some(c) = self.current_char() {
                        if is_var_name_char(c) {
                            name.push(c);
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                Ok(WordPart::Variable {
                    name,
                    expansion_type: ParamExpansion::Normal,
                })
            }
            _ => Ok(WordPart::Literal("$".to_string())),
        }
    }

    fn scan_braced_param(&mut self) -> Result<WordPart> {
        let mut content = String::new();
        while let Some(ch) = self.current_char() {
            if ch == '}' {
                self.advance();
                break;
            }
            content.push(ch);
            self.advance();
        }

        // Parse ${...} variations
        if content.starts_with('#') && content.len() > 1 {
            return Ok(WordPart::Variable {
                name: content[1..].to_string(),
                expansion_type: ParamExpansion::Length,
            });
        }

        if let Some(idx) = content.find(":-") {
            let name = content[..idx].to_string();
            let default = content[idx + 2..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::DefaultValue {
                    default,
                    assign_if_unset: false,
                },
            });
        }

        if let Some(idx) = content.find(":=") {
            let name = content[..idx].to_string();
            let default = content[idx + 2..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::DefaultValue {
                    default,
                    assign_if_unset: true,
                },
            });
        }

        if let Some(idx) = content.find(":+") {
            let name = content[..idx].to_string();
            let alternative = content[idx + 2..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::UseAlternative { alternative },
            });
        }

        if let Some(idx) = content.find(":?") {
            let name = content[..idx].to_string();
            let message = content[idx + 2..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::ErrorIfUnset { message },
            });
        }

        if let Some(idx) = content.find("%%") {
            let name = content[..idx].to_string();
            let pattern = content[idx + 2..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::RemoveSuffix {
                    pattern,
                    longest: true,
                },
            });
        }

        if let Some(idx) = content.find('%') {
            let name = content[..idx].to_string();
            let pattern = content[idx + 1..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::RemoveSuffix {
                    pattern,
                    longest: false,
                },
            });
        }

        if let Some(idx) = content.find("##") {
            let name = content[..idx].to_string();
            let pattern = content[idx + 2..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::RemovePrefix {
                    pattern,
                    longest: true,
                },
            });
        }

        if let Some(idx) = content.find('#') {
            let name = content[..idx].to_string();
            let pattern = content[idx + 1..].to_string();
            return Ok(WordPart::Variable {
                name,
                expansion_type: ParamExpansion::RemovePrefix {
                    pattern,
                    longest: false,
                },
            });
        }

        Ok(WordPart::Variable {
            name: content,
            expansion_type: ParamExpansion::Normal,
        })
    }
}
