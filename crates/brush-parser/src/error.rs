use crate::tokenizer;

/// Represents an error that occurred while parsing tokens.
#[derive(Debug)]
pub enum ParseError {
    /// A parsing error occurred near the given position.
    ParsingNear(crate::SourcePosition),

    /// A parsing error occurred at the end of the input.
    ParsingAtEndOfInput,

    /// An error occurred while tokenizing the input stream.
    Tokenizing {
        /// The inner error.
        inner: tokenizer::TokenizerError,
        /// Optionally provides the position of the error.
        position: Option<crate::SourcePosition>,
    },
}

#[cfg(feature = "diagnostics")]
#[allow(clippy::cast_sign_loss)]
#[allow(unused)] // Workaround unused warnings in nightly versions of the compiler
pub mod miette {
    use super::ParseError;
    use miette::SourceOffset;

    impl ParseError {
        /// Convert the original error to one miette can pretty print
        pub fn to_pretty_error(self, input: impl Into<String>) -> PrettyError {
            let input = input.into();
            let location = match self {
                Self::ParsingNear(ref pos) => {
                    Some(SourceOffset::from_location(&input, pos.line, pos.column))
                }
                Self::Tokenizing { ref position, .. } => position
                    .as_ref()
                    .map(|p| SourceOffset::from_location(&input, p.line, p.column)),
                Self::ParsingAtEndOfInput => {
                    Some(SourceOffset::from_location(&input, usize::MAX, usize::MAX))
                }
            };

            PrettyError {
                cause: self,
                input,
                location,
            }
        }
    }

    /// Represents an error that occurred while parsing tokens.
    #[derive(thiserror::Error, Debug, miette::Diagnostic)]
    pub struct PrettyError {
        cause: ParseError,
        #[source_code]
        input: String,
        #[label("{cause}")]
        location: Option<SourceOffset>,
    }
}

/// Represents a parsing error with its location information
#[derive(Debug)]
pub struct ParseErrorLocation {
    inner: peg::error::ParseError<peg::str::LineCol>,
}

/// Represents an error that occurred while parsing a word.
#[derive(Debug)]
pub enum WordParseError {
    /// An error occurred while parsing an arithmetic expression.
    ArithmeticExpression(ParseErrorLocation),

    /// An error occurred while parsing a shell pattern.
    Pattern(ParseErrorLocation),

    /// An error occurred while parsing a prompt string.
    Prompt(ParseErrorLocation),

    /// An error occurred while parsing a parameter.
    Parameter(String, ParseErrorLocation),

    /// An error occurred while parsing for brace expansion.
    BraceExpansion(String, ParseErrorLocation),

    /// An error occurred while parsing a word.
    Word(String, ParseErrorLocation),
}

/// Represents an error that occurred while parsing a (non-extended) test command.
#[derive(Debug)]
pub struct TestCommandParseError(peg::error::ParseError<usize>);

/// Represents an error that occurred while parsing a key-binding specification.
#[derive(Debug)]
pub enum BindingParseError {
    /// An unknown error occurred while parsing a key-binding specification.
    Unknown(String),

    /// A key code was missing from the key-binding specification.
    MissingKeyCode,
}

pub(crate) fn convert_peg_parse_error(
    err: &peg::error::ParseError<usize>,
    tokens: &[crate::Token],
) -> ParseError {
    let approx_token_index = err.location;

    if approx_token_index < tokens.len() {
        let token = &tokens[approx_token_index];
        ParseError::ParsingNear((*token.location().start).clone())
    } else {
        ParseError::ParsingAtEndOfInput
    }
}

// The `Display` and `Error` impls the `thiserror` derive used to generate.
//
// Three enums and two newtypes, twenty-eight messages. `thiserror` writes exactly this and brings
// `syn` with it — the last duplicate version in the tree, and a proc-macro dylib to build before
// this crate can start. The wording below is the derive's, message for message: these strings are
// what a user sees when a script will not parse.

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ParsingNear(p) => write!(f, "syntax error at line {} col {}", p.line, p.column),
            Self::ParsingAtEndOfInput => write!(f, "syntax error at end of input"),
            Self::Tokenizing { inner, position } => {
                let near = position.as_ref().map_or_else(
                    || String::from("<unknown position>"),
                    |p| std::format!("line {} col {}", p.line, p.column),
                );
                write!(f, "{inner} (detected near {near})")
            }
        }
    }
}

impl std::error::Error for ParseError {}

impl std::fmt::Display for ParseErrorLocation {
    /// `#[error(transparent)]`: the wrapper adds no wording of its own.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(f)
    }
}

impl std::error::Error for ParseErrorLocation {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.inner)
    }
}

impl From<peg::error::ParseError<peg::str::LineCol>> for ParseErrorLocation {
    fn from(inner: peg::error::ParseError<peg::str::LineCol>) -> Self {
        Self { inner }
    }
}

impl std::fmt::Display for WordParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArithmeticExpression(_) => write!(f, "failed to parse arithmetic expression"),
            Self::Pattern(_) => write!(f, "failed to parse pattern"),
            Self::Prompt(_) => write!(f, "failed to parse prompt string"),
            Self::Parameter(name, _) => write!(f, "failed to parse parameter '{name}'"),
            Self::BraceExpansion(word, _) => {
                write!(f, "failed to parse for brace expansion: '{word}'")
            }
            Self::Word(word, _) => write!(f, "failed to parse word '{word}'"),
        }
    }
}

impl std::error::Error for WordParseError {}

impl std::fmt::Display for TestCommandParseError {
    /// `#[error(transparent)]`, as above.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for TestCommandParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

impl From<peg::error::ParseError<usize>> for TestCommandParseError {
    fn from(inner: peg::error::ParseError<usize>) -> Self {
        Self(inner)
    }
}

impl std::fmt::Display for BindingParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(what) => write!(f, "unknown error while parsing key-binding: '{what}'"),
            Self::MissingKeyCode => write!(f, "missing key code in key-binding"),
        }
    }
}

impl std::error::Error for BindingParseError {}
