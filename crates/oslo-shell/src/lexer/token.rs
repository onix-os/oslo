use oslo_base::ast::Word;

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

    // No reserved words: the shell's grammar is brush-parser's, and this lexer only ever sees an
    // array literal or a declaration payload, where `do` and `in` are ordinary elements. The
    // fifteen variants that used to sit here were constructed and never read — see `scan_word`.
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
