use crate::ast::*;
use crate::error::{Result, ShellError};
use crate::lexer::{Lexer, Token};

pub struct Parser<'a> {
    lexer: Lexer<'a>,
}

impl<'a> Parser<'a> {
    pub fn new(lexer: Lexer<'a>) -> Self {
        Self { lexer }
    }

    pub fn parse_command_list(&mut self) -> Result<CommandList> {
        let mut items = Vec::new();

        self.skip_newlines()?;

        while !matches!(
            self.lexer.peek()?,
            Token::Eof
                | Token::RParen
                | Token::RBrace
                | Token::Fi
                | Token::Done
                | Token::Do
                | Token::Esac
                | Token::Else
                | Token::Elif
                | Token::Then
        ) {
            let and_or = self.parse_and_or_list()?;

            let op = match self.lexer.peek()? {
                Token::Semicolon => {
                    self.lexer.next()?;
                    ListOp::Sequential
                }
                Token::Amp => {
                    self.lexer.next()?;
                    ListOp::Background
                }
                Token::Newline => {
                    self.lexer.next()?;
                    ListOp::Newline
                }
                _ => ListOp::Sequential,
            };

            items.push(ListItem { and_or, op });
            self.skip_newlines()?;
        }

        Ok(CommandList { items })
    }

    fn skip_newlines(&mut self) -> Result<()> {
        while matches!(self.lexer.peek()?, Token::Newline) {
            self.lexer.next()?;
        }
        Ok(())
    }

    pub fn parse_and_or_list(&mut self) -> Result<AndOrList> {
        let first = self.parse_pipeline()?;
        let mut rest = Vec::new();

        loop {
            let op = match self.lexer.peek()? {
                Token::AndIf => {
                    self.lexer.next()?;
                    AndOrOp::And
                }
                Token::OrIf => {
                    self.lexer.next()?;
                    AndOrOp::Or
                }
                _ => break,
            };

            self.skip_newlines()?;
            let next_pipeline = self.parse_pipeline()?;
            rest.push((op, next_pipeline));
        }

        Ok(AndOrList { first, rest })
    }

    pub fn parse_pipeline(&mut self) -> Result<Pipeline> {
        let mut negated = false;
        if matches!(self.lexer.peek()?, Token::Word(w) if w.parts.len() == 1 && w.parts[0] == WordPart::Literal("!".to_string()))
        {
            self.lexer.next()?;
            negated = true;
        }

        let mut commands = vec![self.parse_command()?];

        while matches!(self.lexer.peek()?, Token::Pipe) {
            self.lexer.next()?;
            self.skip_newlines()?;
            commands.push(self.parse_command()?);
        }

        Ok(Pipeline { negated, commands })
    }

    pub fn parse_command(&mut self) -> Result<Command> {
        let peeked = self.lexer.peek()?.clone();

        match peeked {
            Token::If => self.parse_if(),
            Token::While => self.parse_while(),
            Token::Until => self.parse_until(),
            Token::For => self.parse_for(),
            Token::Case => self.parse_case(),
            Token::LParen => self.parse_subshell(),
            Token::LBrace => self.parse_group(),
            Token::Word(_)
            | Token::IoNumber(_)
            | Token::Less
            | Token::Great
            | Token::Dgreat
            | Token::Dless
            | Token::DlessDash
            | Token::LessAnd
            | Token::GreatAnd
            | Token::LessGreat
            | Token::Clobber => {
                if let Token::Word(ref w) = peeked
                    && w.parts.len() == 1
                    && let WordPart::Literal(ref name) = w.parts[0]
                {
                    let fn_name = name.clone();
                    let cmd = self.parse_simple_command()?;
                    if cmd.words.len() == 1 && matches!(self.lexer.peek()?, Token::LParen) {
                        self.lexer.next()?; // (
                        if matches!(self.lexer.peek()?, Token::RParen) {
                            self.lexer.next()?; // )
                            self.skip_newlines()?;
                            let body = self.parse_command()?;
                            return Ok(Command::FunctionDef {
                                name: fn_name,
                                body: Box::new(body),
                            });
                        }
                    }
                    return Ok(Command::Simple(cmd));
                }
                let simple = self.parse_simple_command()?;
                Ok(Command::Simple(simple))
            }
            _ => Err(ShellError::SyntaxError(format!(
                "Unexpected token: {:?}",
                peeked
            ))),
        }
    }

    fn parse_if(&mut self) -> Result<Command> {
        self.lexer.next()?; // if
        let condition = self.parse_command_list()?;

        if !matches!(self.lexer.next()?, Token::Then) {
            return Err(ShellError::SyntaxError(
                "Expected 'then' after if condition".to_string(),
            ));
        }

        let then_branch = self.parse_command_list()?;
        let mut elif_branches = Vec::new();
        let mut else_branch = None;

        loop {
            match self.lexer.peek()? {
                Token::Elif => {
                    self.lexer.next()?;
                    let elif_cond = self.parse_command_list()?;
                    if !matches!(self.lexer.next()?, Token::Then) {
                        return Err(ShellError::SyntaxError(
                            "Expected 'then' after elif condition".to_string(),
                        ));
                    }
                    let elif_body = self.parse_command_list()?;
                    elif_branches.push((elif_cond, elif_body));
                }
                Token::Else => {
                    self.lexer.next()?;
                    else_branch = Some(self.parse_command_list()?);
                    break;
                }
                _ => break,
            }
        }

        if !matches!(self.lexer.next()?, Token::Fi) {
            return Err(ShellError::SyntaxError(
                "Expected 'fi' to end if statement".to_string(),
            ));
        }

        let redirections = self.parse_redirections()?;

        Ok(Command::Compound {
            kind: CompoundCommand::If {
                condition,
                then_branch,
                elif_branches,
                else_branch,
            },
            redirections,
        })
    }

    fn parse_while(&mut self) -> Result<Command> {
        self.lexer.next()?; // while
        let condition = self.parse_command_list()?;

        if !matches!(self.lexer.next()?, Token::Do) {
            return Err(ShellError::SyntaxError(
                "Expected 'do' after while condition".to_string(),
            ));
        }

        let body = self.parse_command_list()?;

        if !matches!(self.lexer.next()?, Token::Done) {
            return Err(ShellError::SyntaxError(
                "Expected 'done' to end while loop".to_string(),
            ));
        }

        let redirections = self.parse_redirections()?;

        Ok(Command::Compound {
            kind: CompoundCommand::While { condition, body },
            redirections,
        })
    }

    fn parse_until(&mut self) -> Result<Command> {
        self.lexer.next()?; // until
        let condition = self.parse_command_list()?;

        if !matches!(self.lexer.next()?, Token::Do) {
            return Err(ShellError::SyntaxError(
                "Expected 'do' after until condition".to_string(),
            ));
        }

        let body = self.parse_command_list()?;

        if !matches!(self.lexer.next()?, Token::Done) {
            return Err(ShellError::SyntaxError(
                "Expected 'done' to end until loop".to_string(),
            ));
        }

        let redirections = self.parse_redirections()?;

        Ok(Command::Compound {
            kind: CompoundCommand::Until { condition, body },
            redirections,
        })
    }

    fn parse_for(&mut self) -> Result<Command> {
        self.lexer.next()?; // for

        let var_name = match self.lexer.next()? {
            Token::Word(w) => match w.parts.first() {
                Some(WordPart::Literal(s)) => s.clone(),
                _ => {
                    return Err(ShellError::SyntaxError(
                        "Expected variable name after 'for'".to_string(),
                    ));
                }
            },
            _ => {
                return Err(ShellError::SyntaxError(
                    "Expected variable name after 'for'".to_string(),
                ));
            }
        };

        self.skip_newlines()?;

        let items = if matches!(self.lexer.peek()?, Token::In) {
            self.lexer.next()?; // in
            let mut words = Vec::new();
            while let Token::Word(w) = self.lexer.peek()?.clone() {
                self.lexer.next()?;
                words.push(w);
            }
            if matches!(self.lexer.peek()?, Token::Semicolon | Token::Newline) {
                self.lexer.next()?;
            }
            Some(words)
        } else {
            if matches!(self.lexer.peek()?, Token::Semicolon | Token::Newline) {
                self.lexer.next()?;
            }
            None
        };

        self.skip_newlines()?;

        if !matches!(self.lexer.next()?, Token::Do) {
            return Err(ShellError::SyntaxError(
                "Expected 'do' in for loop".to_string(),
            ));
        }

        let body = self.parse_command_list()?;

        if !matches!(self.lexer.next()?, Token::Done) {
            return Err(ShellError::SyntaxError(
                "Expected 'done' to end for loop".to_string(),
            ));
        }

        let redirections = self.parse_redirections()?;

        Ok(Command::Compound {
            kind: CompoundCommand::For {
                var_name,
                items,
                body,
            },
            redirections,
        })
    }

    fn parse_case(&mut self) -> Result<Command> {
        self.lexer.next()?; // case
        let word = match self.lexer.next()? {
            Token::Word(w) => w,
            _ => {
                return Err(ShellError::SyntaxError(
                    "Expected word after 'case'".to_string(),
                ));
            }
        };

        self.skip_newlines()?;

        if !matches!(self.lexer.next()?, Token::In) {
            return Err(ShellError::SyntaxError(
                "Expected 'in' in case statement".to_string(),
            ));
        }

        self.skip_newlines()?;
        let mut items = Vec::new();

        while !matches!(self.lexer.peek()?, Token::Esac | Token::Eof) {
            if matches!(self.lexer.peek()?, Token::LParen) {
                self.lexer.next()?;
            }

            let mut patterns = Vec::new();
            loop {
                if let Token::Word(w) = self.lexer.next()? {
                    patterns.push(w);
                } else {
                    return Err(ShellError::SyntaxError(
                        "Expected pattern in case item".to_string(),
                    ));
                }

                if matches!(self.lexer.peek()?, Token::RParen) {
                    self.lexer.next()?;
                    break;
                } else if matches!(self.lexer.peek()?, Token::Pipe) {
                    self.lexer.next()?;
                } else {
                    return Err(ShellError::SyntaxError(
                        "Expected ')' or '|' after case pattern".to_string(),
                    ));
                }
            }

            let body = self.parse_command_list()?;
            items.push(CaseItem { patterns, body });

            if matches!(self.lexer.peek()?, Token::Dsemi) {
                self.lexer.next()?;
                self.skip_newlines()?;
            }
        }

        if !matches!(self.lexer.next()?, Token::Esac) {
            return Err(ShellError::SyntaxError(
                "Expected 'esac' to end case statement".to_string(),
            ));
        }

        let redirections = self.parse_redirections()?;

        Ok(Command::Compound {
            kind: CompoundCommand::Case { word, items },
            redirections,
        })
    }

    fn parse_subshell(&mut self) -> Result<Command> {
        self.lexer.next()?; // (
        let body = self.parse_command_list()?;
        if !matches!(self.lexer.next()?, Token::RParen) {
            return Err(ShellError::SyntaxError(
                "Expected ')' to close subshell".to_string(),
            ));
        }
        let redirections = self.parse_redirections()?;
        Ok(Command::Compound {
            kind: CompoundCommand::Subshell(body),
            redirections,
        })
    }

    fn parse_group(&mut self) -> Result<Command> {
        self.lexer.next()?; // {
        let body = self.parse_command_list()?;
        if !matches!(self.lexer.next()?, Token::RBrace) {
            return Err(ShellError::SyntaxError(
                "Expected '}' to close command group".to_string(),
            ));
        }
        let redirections = self.parse_redirections()?;
        Ok(Command::Compound {
            kind: CompoundCommand::Group(body),
            redirections,
        })
    }

    fn parse_simple_command(&mut self) -> Result<SimpleCommand> {
        let mut assignments = Vec::new();
        let mut words = Vec::new();
        let mut redirections = Vec::new();

        loop {
            let peeked = self.lexer.peek()?.clone();

            // Check redirections
            if is_redirection_start(&peeked) {
                let redir = self.parse_redirection()?;
                redirections.push(redir);
                continue;
            }

            match peeked {
                Token::Word(ref w) => {
                    if words.is_empty()
                        && let Some(assign) = try_parse_assignment(w)
                    {
                        self.lexer.next()?;
                        assignments.push(assign);
                        continue;
                    }
                    self.lexer.next()?;
                    words.push(w.clone());
                }
                _ => break,
            }
        }

        if assignments.is_empty() && words.is_empty() && redirections.is_empty() {
            return Err(ShellError::SyntaxError("Empty simple command".to_string()));
        }

        Ok(SimpleCommand {
            assignments,
            words,
            redirections,
        })
    }

    fn parse_redirections(&mut self) -> Result<Vec<Redirection>> {
        let mut redirections = Vec::new();
        while is_redirection_start(self.lexer.peek()?) {
            redirections.push(self.parse_redirection()?);
        }
        Ok(redirections)
    }

    fn parse_redirection(&mut self) -> Result<Redirection> {
        let mut explicit_fd = None;
        if let Token::IoNumber(num) = self.lexer.peek()? {
            explicit_fd = Some(*num);
            self.lexer.next()?;
        }

        let op_tok = self.lexer.next()?;
        let (kind, default_fd) = match op_tok {
            Token::Less => (RedirectKind::Input, 0),
            Token::Great => (RedirectKind::Output, 1),
            Token::Dgreat => (RedirectKind::Append, 1),
            Token::Dless => (RedirectKind::Heredoc, 0),
            Token::DlessDash => (RedirectKind::HeredocStrip, 0),
            Token::LessAnd => (RedirectKind::DupInput, 0),
            Token::GreatAnd => (RedirectKind::DupOutput, 1),
            Token::LessGreat => (RedirectKind::ReadWrite, 0),
            Token::Clobber => (RedirectKind::Clobber, 1),
            _ => {
                return Err(ShellError::SyntaxError(
                    "Expected redirection operator".to_string(),
                ));
            }
        };

        let target = match self.lexer.next()? {
            Token::Word(w) => w,
            tok => {
                return Err(ShellError::SyntaxError(format!(
                    "Expected redirection target, got {:?}",
                    tok
                )));
            }
        };

        Ok(Redirection {
            fd: explicit_fd.or(Some(default_fd)),
            kind,
            target,
            heredoc_content: None,
        })
    }
}

fn is_redirection_start(tok: &Token) -> bool {
    matches!(
        tok,
        Token::IoNumber(_)
            | Token::Less
            | Token::Great
            | Token::Dgreat
            | Token::Dless
            | Token::DlessDash
            | Token::LessAnd
            | Token::GreatAnd
            | Token::LessGreat
            | Token::Clobber
    )
}

fn try_parse_assignment(w: &Word) -> Option<Assignment> {
    if let Some(WordPart::Literal(s)) = w.parts.first()
        && let Some(idx) = s.find('=')
    {
        let name = s[..idx].to_string();
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let first_val_str = s[idx + 1..].to_string();
            let mut val_parts = Vec::new();
            if !first_val_str.is_empty() {
                val_parts.push(WordPart::Literal(first_val_str));
            }
            val_parts.extend(w.parts[1..].iter().cloned());

            return Some(Assignment {
                name,
                value: Word { parts: val_parts },
            });
        }
    }
    None
}
