use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShellError {
    #[error("Syntax error: {0}")]
    SyntaxError(String),

    #[error("Expansion error: {0}")]
    ExpansionError(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Lua error: {0}")]
    Lua(#[from] mlua::Error),

    #[error("POSIX error: {0}")]
    Nix(#[from] nix::Error),

    #[error("Builtin exit requested with code: {0}")]
    Exit(i32),

    #[error("Return called with code: {0}")]
    Return(i32),

    #[error("Break called with depth: {0}")]
    Break(usize),

    #[error("Continue called with depth: {0}")]
    Continue(usize),
}

pub type Result<T> = std::result::Result<T, ShellError>;
