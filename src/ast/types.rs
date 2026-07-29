#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WordPart {
    Literal(String),
    /// Characters the lexer stripped a backslash from.
    ///
    /// Distinct from [`WordPart::Literal`] because escaping is a form of quoting: `\*` must not
    /// glob and `a\ b` must not field-split, neither of which a literal can express. Kept as its
    /// own variant rather than a flag on `Literal` so that every match on a word part has to
    /// decide what escaping means for it.
    Escaped(String),
    SingleQuoted(String),
    DoubleQuoted(Vec<WordPart>),
    Variable {
        name: String,
        expansion_type: ParamExpansion,
    },
    CommandSubstitution(String),
    Arithmetic(String),
    Tilde(String),
}

/// The operator inside a `${...}`, with its operand still unexpanded.
///
/// Every operand is a [`Word`], not a `String`: `${x:-$HOME}` has to expand its default, and
/// `${x:=$y}` has to *assign* the expanded text. Storing the raw source here and expanding it in
/// `crate::expand::param` is what keeps `$HOME` from being persisted literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamExpansion {
    Normal,
    /// `${#name}` — character length, or the positional count for `#`, `@` and `*`.
    Length,
    /// `${name:-word}`, `${name-word}`, and the assigning `:=` / `=` forms.
    DefaultValue {
        default: Word,
        assign_if_unset: bool,
        /// The `:` variants also treat a set-but-empty parameter as absent; the colon-less ones
        /// test only for *unset*, which is the whole difference between `${x-d}` and `${x:-d}`.
        test_null: bool,
    },
    UseAlternative {
        alternative: Word,
        test_null: bool,
    },
    ErrorIfUnset {
        message: Word,
        test_null: bool,
    },
    /// `${name%pat}` / `${name%%pat}`.
    RemoveSuffix {
        pattern: Word,
        longest: bool,
    },
    /// `${name#pat}` / `${name##pat}`.
    RemovePrefix {
        pattern: Word,
        longest: bool,
    },
    /// `${name:offset}` / `${name:offset:length}`; both operands are arithmetic.
    Substring {
        offset: Word,
        length: Option<Word>,
    },
    /// `${name/pat/rep}` and its `//`, `/#`, `/%` variants.
    Replace {
        pattern: Word,
        replacement: Word,
        scope: ReplaceScope,
    },
    /// `${name^}`, `${name^^}`, `${name,}`, `${name,,}`.
    CaseConvert {
        /// Which characters are eligible; `None` means every character.
        pattern: Option<Word>,
        upper: bool,
        /// Doubled operator: convert every eligible character rather than just the first.
        all: bool,
    },
    /// `${!name}` — `name` holds the name of the parameter to expand.
    Indirect,
}

/// Which occurrences `${name/pat/rep}` replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceScope {
    /// `/` — the leftmost match.
    First,
    /// `//` — every match.
    All,
    /// `/#` — only a match anchored at the start.
    Prefix,
    /// `/%` — only a match anchored at the end.
    Suffix,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Word {
    pub parts: Vec<WordPart>,
}

impl Word {
    pub fn from_literal(s: &str) -> Self {
        Self {
            parts: vec![WordPart::Literal(s.to_string())],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub name: String,
    pub value: Word,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectKind {
    Input,        // <
    Output,       // >
    Append,       // >>
    ReadWrite,    // <>
    DupInput,     // <&
    DupOutput,    // >&
    Heredoc,      // <<
    HeredocStrip, // <<-
    Clobber,      // >|
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirection {
    pub fd: Option<i32>,
    pub kind: RedirectKind,
    pub target: Word,
    pub heredoc_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    pub assignments: Vec<Assignment>,
    pub words: Vec<Word>,
    pub redirections: Vec<Redirection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Simple(SimpleCommand),
    Compound {
        kind: CompoundCommand,
        redirections: Vec<Redirection>,
    },
    FunctionDef {
        name: String,
        body: Box<Command>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompoundCommand {
    If {
        condition: CommandList,
        then_branch: CommandList,
        elif_branches: Vec<(CommandList, CommandList)>,
        else_branch: Option<CommandList>,
    },
    While {
        condition: CommandList,
        body: CommandList,
    },
    Until {
        condition: CommandList,
        body: CommandList,
    },
    For {
        var_name: String,
        items: Option<Vec<Word>>,
        body: CommandList,
    },
    Case {
        word: Word,
        items: Vec<CaseItem>,
    },
    Subshell(CommandList),
    Group(CommandList),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseItem {
    pub patterns: Vec<Word>,
    pub body: CommandList,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    pub negated: bool,
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AndOrOp {
    And, // &&
    Or,  // ||
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndOrList {
    pub first: Pipeline,
    pub rest: Vec<(AndOrOp, Pipeline)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOp {
    Sequential, // ;
    Background, // &
    Newline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    pub and_or: AndOrList,
    pub op: ListOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandList {
    pub items: Vec<ListItem>,
}
