use crate::ast::Word;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    // Words and IoNumbers
    Word(Word),
    IoNumber(i32),

    // Operators
    Pipe,      // |
    Amp,       // &
    Semicolon, // ;
    AndIf,     // &&
    OrIf,      // ||
    Dsemi,     // ;;
    Less,      // <
    Great,     // >
    Dgreat,    // >>
    Dless,     // <<
    DlessDash, // <<-
    LessAnd,   // <&
    GreatAnd,  // >&
    LessGreat, // <>
    Clobber,   // >|

    // Reserved Words
    If,
    Then,
    Else,
    Elif,
    Fi,
    Case,
    Esac,
    For,
    While,
    Until,
    Do,
    Done,
    In,
    LBrace, // {
    RBrace, // }
    LParen, // (
    RParen, // )

    // Structure
    Newline,
    Eof,
}

impl Token {
    pub fn is_operator(&self) -> bool {
        matches!(
            self,
            Token::Pipe
                | Token::Amp
                | Token::Semicolon
                | Token::AndIf
                | Token::OrIf
                | Token::Dsemi
                | Token::Less
                | Token::Great
                | Token::Dgreat
                | Token::Dless
                | Token::DlessDash
                | Token::LessAnd
                | Token::GreatAnd
                | Token::LessGreat
                | Token::Clobber
                | Token::Newline
                | Token::Eof
        )
    }
}
