//! The one error type the evaluator unwinds with, and how it becomes an exit status.
//!
//! Four of the variants are not failures at all: `exit`, `return`, `break` and `continue` travel
//! as errors only because that is how a value escapes an arbitrarily deep evaluation. Anything
//! that turns an error into a number has to tell the two kinds apart, which is what
//! [`ShellError::control_flow_status`] is for — collapsing them all to 1 is exactly the bug
//! behind `( exit 3 )` reporting 1.

/// Written out rather than derived.
///
/// `thiserror` was one derive, in this one file, and it brings `syn`, `quote` and `proc-macro2`
/// into every build of the crate — three of the slowest dependencies there are, compiled before
/// anything else can start, to generate the sixty lines below. A shell with one error type does
/// not need a macro to write them.
#[derive(Debug)]
pub enum ShellError {
    SyntaxError(String),

    ExpansionError(String),

    ExecutionError(String),

    Io(std::io::Error),

    Lua(oslo_lua::LuaError),

    Nix(nix::Error),

    Exit(i32),

    Return(i32),

    Break(usize),

    Continue(usize),

    /// A **utility error**: the command failed in one of the ways POSIX 2.8.1 calls fatal to a
    /// non-interactive POSIX-mode shell — a bad option, a bad operand, a variable-assignment
    /// error or a redirection error — as opposed to an ordinary non-zero exit status.
    ///
    /// The distinction is the whole point, and bash draws it exactly here:
    ///
    /// ```text
    /// bash --posix -c 'shift 5; echo alive'            -> diagnostic, "alive", status 0
    /// bash --posix -c 'export BAD-NAME=1; echo alive'  -> diagnostic, no "alive", status 1
    /// ```
    ///
    /// `shift 5` is an ordinary failure; `export BAD-NAME=1` is a utility error. Builtins return
    /// `Result<i32>` and a usage error is an indistinguishable `Ok(2)`, so a shell that keyed off
    /// "status != 0" would kill itself on `shift`. This variant is the marker that tells them
    /// apart; `crate::exec::simple::posix` is the only place that acts on it.
    ///
    /// **The diagnostic is already on stderr when this is raised.** Whoever detected the error
    /// printed it, in the wording that error deserves; `context` exists so that a path which
    /// renders the error anyway says something true rather than nothing.
    UtilityError {
        /// What went wrong, for rendering only — never the primary report.
        context: String,
        /// The status the failed *command* takes on where the shell carries on.
        status: i32,
        /// The status the *shell* exits with where POSIX says it must not carry on.
        fatal: i32,
    },
}

/// An `io::Error`'s reason, without the number Rust appends to it.
///
/// **`Display` for `io::Error` ends in ` (os error 30)`, and no shell prints that.** These strings
/// reach the user as `oslo: /etc/thing: Read-only file system`, which is the shape every other shell
/// uses and the shape that ends up in a package build log; the errno is an implementation detail of
/// the language oslo happens to be written in.
///
/// Found by running 816 Ubuntu maintainer scripts under `dash` and under oslo and diffing: nothing
/// *behaved* differently, and this was most of what the diff was full of.
pub fn reason(e: &std::io::Error) -> String {
    let said = e.to_string();
    let Some(at) = said.rfind(" (os error ") else {
        return said;
    };
    // **The number has to be a number.** Trimming on the phrase alone turned an error that merely
    // mentioned it — `failed (os error reading the manual)` — into `failed`, which is a shell
    // swallowing the half of a message that said what happened. Its own test caught that.
    let inside = &said[at + " (os error ".len()..];
    match inside.strip_suffix(')') {
        Some(digits) if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
            said[..at].to_string()
        }
        _ => said,
    }
}

impl std::fmt::Display for ShellError {
    /// The wording reaches the user through `oslo: {error}`, so it is part of the interface.
    ///
    /// **`ExecutionError` prints no category of its own.** Its message is already
    /// `what: why` — `/etc/default/alsa: Read-only file system` — and bash writes exactly that.
    /// "Execution error: " in front of it named the enum variant rather than telling anybody
    /// anything. `Syntax error` and `Expansion error` keep theirs: those *are* what went wrong, and
    /// two tests use the first as the signal that a script parsed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellError::SyntaxError(m) => write!(f, "Syntax error: {m}"),
            ShellError::ExpansionError(m) => write!(f, "Expansion error: {m}"),
            ShellError::ExecutionError(m) => write!(f, "{m}"),
            ShellError::Io(e) => write!(f, "{}", reason(e)),
            ShellError::Lua(e) => write!(f, "Lua error: {e}"),
            ShellError::Nix(e) => write!(f, "POSIX error: {e}"),
            ShellError::Exit(c) => write!(f, "Builtin exit requested with code: {c}"),
            ShellError::Return(c) => write!(f, "Return called with code: {c}"),
            ShellError::Break(d) => write!(f, "Break called with depth: {d}"),
            ShellError::Continue(d) => write!(f, "Continue called with depth: {d}"),
            ShellError::UtilityError { context, .. } => write!(f, "{context}"),
        }
    }
}

impl std::error::Error for ShellError {
    /// The three wrapped errors keep their chain, which is what `#[from]` gave them. Everything
    /// else carries a `String` and has no source to point at.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ShellError::Io(e) => Some(e),
            ShellError::Lua(e) => Some(e),
            ShellError::Nix(e) => Some(e),
            _ => None,
        }
    }
}

// The three `#[from]` conversions, so `?` still works on the errors the shell actually propagates.
impl From<std::io::Error> for ShellError {
    fn from(e: std::io::Error) -> Self {
        ShellError::Io(e)
    }
}

impl From<oslo_lua::LuaError> for ShellError {
    fn from(e: oslo_lua::LuaError) -> Self {
        ShellError::Lua(e)
    }
}

impl From<nix::Error> for ShellError {
    fn from(e: nix::Error) -> Self {
        ShellError::Nix(e)
    }
}

impl ShellError {
    /// A utility error raised by a builtin: `status` is what the command reports, and it is also
    /// what the shell exits with in POSIX mode — `bash --posix -c 'export BAD-NAME=1'` exits 1,
    /// which is `export`'s own status, and `set -o nosuchopt` exits 2, which is `set`'s.
    pub fn utility_error(context: impl Into<String>, status: i32) -> Self {
        ShellError::UtilityError {
            context: context.into(),
            status,
            fatal: status,
        }
    }

    /// A *variable assignment error*: `r=2` where `r` is read-only, or a name `environ` cannot
    /// hold.
    ///
    /// The two statuses genuinely differ here, which is why the variant carries both. The failed
    /// assignment is worth 1 (`bash -c $'readonly r=1\nr=2\necho "$?"'` prints 1), but a POSIX
    /// shell does not carry on at all, and it abandons its program with the status every other
    /// abandoned program gets — see [`ShellError::fatal_exit_status`].
    pub fn assignment_error(context: impl Into<String>) -> Self {
        ShellError::UtilityError {
            context: context.into(),
            status: 1,
            fatal: FATAL_EXIT_STATUS,
        }
    }

    /// The status this error *carries*, for the variants that are control flow rather than
    /// failure — `None` for a genuine error.
    ///
    /// `exit n` and `return n` carry `n`. `break`/`continue` carry nothing: when one escapes the
    /// last enclosing loop there is no loop left to complain to, and every POSIX shell treats it
    /// as a silent no-op with the status of whatever ran before it, so 0 here.
    pub fn control_flow_status(&self) -> Option<i32> {
        match self {
            ShellError::Exit(code) | ShellError::Return(code) => Some(*code),
            ShellError::Break(_) | ShellError::Continue(_) => Some(0),
            _ => None,
        }
    }

    /// The status a genuine failure reports where it is caught.
    ///
    /// 2 for a syntax error — the number POSIX reserves for one and the number `make` and CI
    /// scripts test for — and 1 for everything else. A *fatal* expansion error aborting a
    /// non-interactive shell is a different question, answered by `main`: bash exits 127 there,
    /// but the same error inside a subshell or a pipeline stage still reports 1.
    pub fn failure_status(&self) -> i32 {
        match self {
            ShellError::SyntaxError(_) => 2,
            ShellError::UtilityError { status, .. } => *status,
            _ => 1,
        }
    }

    /// The status a fatal error gives a *non-interactive* shell that had to abandon its script.
    ///
    /// An expansion that cannot be performed at all — `${v:?msg}`, a malformed `${...}`, division
    /// by zero, a syntax error in the body of a `$( )` — ends the shell with 127 rather than the
    /// 1 the same failure reports from inside a subshell. That is what bash does and what scripts
    /// testing "the shell gave up" look for. A syntax error counts here because the script itself
    /// parsed before it ran: the only way to raise one *during* execution is a `$( )` body, which
    /// bash also parses late and also reports as 127.
    ///
    /// The documented exception is `${!name}` through a value that is not a name, which bash
    /// unwinds by a different path and exits 1 for. Recognising it by its message is a stopgap —
    /// the honest fix is a distinct variant raised where `expand::param` handles indirection —
    /// but that message is built in exactly one place, and the alternative is knowingly reporting
    /// the wrong number.
    /// The status a [`ShellError::UtilityError`] leaves behind when the caller has decided the
    /// shell survives it — `None` for every other error.
    ///
    /// The point is the *diagnostic*: a utility error's message is already on stderr, so a caller
    /// that let it reach `exec::pipeline::report_error_status` would print it twice.
    /// `command` and `builtin` reach a builtin without going through
    /// `crate::exec::simple::posix`, and POSIX 2.9.1.1 says `command` strips a special builtin of
    /// exactly the property that would have made the error fatal, so folding here is also the
    /// right answer and not merely the quiet one.
    pub fn survivable_utility_status(&self) -> Option<i32> {
        match self {
            ShellError::UtilityError { status, .. } => Some(*status),
            _ => None,
        }
    }

    pub fn fatal_exit_status(&self) -> i32 {
        match self {
            ShellError::ExpansionError(msg) if msg.ends_with(INVALID_NAME_SUFFIX) => 1,
            ShellError::SyntaxError(_) | ShellError::ExpansionError(_) => FATAL_EXIT_STATUS,
            ShellError::UtilityError { fatal, .. } => *fatal,
            other => other.failure_status(),
        }
    }
}

/// The status a non-interactive shell reports when it abandons the program it was given.
///
/// Named because three unrelated failures share it and must keep sharing it: a fatal expansion
/// error, a syntax error raised from a `$( )` body, and a POSIX-mode variable assignment error.
/// `bash -c` answers 127 for all three.
pub const FATAL_EXIT_STATUS: i32 = 127;

/// Tail of the diagnostic `expand::param` raises for `${!name}` when `name`'s value is not a
/// name. Lives here because [`ShellError::fatal_exit_status`] is its only reader.
pub const INVALID_NAME_SUFFIX: &str = ": invalid variable name";

pub type Result<T> = std::result::Result<T, ShellError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_and_return_carry_their_code() {
        assert_eq!(ShellError::Exit(3).control_flow_status(), Some(3));
        assert_eq!(ShellError::Exit(255).control_flow_status(), Some(255));
        assert_eq!(ShellError::Return(2).control_flow_status(), Some(2));
    }

    #[test]
    fn loop_control_is_a_silent_zero() {
        assert_eq!(ShellError::Break(1).control_flow_status(), Some(0));
        assert_eq!(ShellError::Continue(4).control_flow_status(), Some(0));
    }

    #[test]
    fn real_errors_carry_no_status() {
        assert_eq!(
            ShellError::ExpansionError("x".into()).control_flow_status(),
            None
        );
        assert_eq!(ShellError::SyntaxError("x".into()).failure_status(), 2);
        assert_eq!(ShellError::ExpansionError("x".into()).failure_status(), 1);
        assert_eq!(ShellError::ExecutionError("x".into()).failure_status(), 1);
    }

    #[test]
    fn a_fatal_expansion_gives_up_with_127() {
        // The same error is worth 1 where it is only a failed command, and 127 where it ends the
        // shell — the two must not be conflated.
        let err = ShellError::ExpansionError("v: is unset".into());
        assert_eq!(err.failure_status(), 1);
        assert_eq!(err.fatal_exit_status(), 127);
        assert_eq!(
            ShellError::SyntaxError("in a $( ) body".into()).fatal_exit_status(),
            127
        );
    }

    #[test]
    fn an_invalid_indirect_name_gives_up_with_1() {
        let err = ShellError::ExpansionError(format!("not a name{}", INVALID_NAME_SUFFIX));
        assert_eq!(err.fatal_exit_status(), 1);
    }

    /// A builtin's utility error reports the builtin's own status on both paths: `export` fails
    /// with 1 and ends a POSIX shell with 1, `set` with 2 and 2.
    #[test]
    fn a_builtin_utility_error_reports_one_status() {
        let err = ShellError::utility_error("export: `BAD-NAME=1': not a valid identifier", 1);
        assert_eq!(err.failure_status(), 1);
        assert_eq!(err.fatal_exit_status(), 1);
        assert_eq!(ShellError::utility_error("x", 2).fatal_exit_status(), 2);
    }

    /// An assignment error reports two: the command failed (1), but the shell gave up (127).
    /// Collapsing them is how `readonly r=1; r=2` ends up reporting the wrong number on one side
    /// or the other.
    #[test]
    fn an_assignment_error_reports_two_statuses() {
        let err = ShellError::assignment_error("r: is read only");
        assert_eq!(err.failure_status(), 1);
        assert_eq!(err.fatal_exit_status(), FATAL_EXIT_STATUS);
        assert_eq!(err.control_flow_status(), None);
        assert_eq!(err.to_string(), "r: is read only");
    }

    #[test]
    fn a_non_expansion_failure_keeps_its_own_status() {
        assert_eq!(
            ShellError::ExecutionError("x".into()).fatal_exit_status(),
            1
        );
    }

    /// **No shell prints an errno.** These strings end up in package build logs, and
    /// `Permission denied (os error 13)` there says oslo is written in Rust rather than saying
    /// what went wrong.
    #[test]
    fn an_io_reason_carries_no_errno() {
        let denied = std::io::Error::from_raw_os_error(13);
        assert!(denied.to_string().contains("os error"), "the leak is real");
        assert_eq!(reason(&denied), "Permission denied");

        let missing = std::io::Error::from_raw_os_error(2);
        assert_eq!(reason(&missing), "No such file or directory");
    }

    /// An error that never had a code keeps every word of what it said.
    #[test]
    fn an_error_without_an_errno_is_left_alone() {
        let made_up = std::io::Error::other("the tape ran out");
        assert_eq!(reason(&made_up), "the tape ran out");
        // And a message that merely mentions the words is not truncated.
        let awkward = std::io::Error::other("failed (os error reading the manual)");
        assert_eq!(reason(&awkward), "failed (os error reading the manual)");
    }

    /// **`ExecutionError` names no category.** Its message is already `what: why`, which is what
    /// bash writes; the variant's name in front of it told nobody anything.
    #[test]
    fn an_execution_error_reads_the_way_bash_writes_one() {
        let failed = ShellError::ExecutionError("/etc/thing: Read-only file system".to_string());
        assert_eq!(failed.to_string(), "/etc/thing: Read-only file system");

        // The two that *are* a category keep it — and a test elsewhere uses the first as its
        // signal that a script parsed.
        let syntax = ShellError::SyntaxError("unexpected end of input".to_string());
        assert!(syntax.to_string().starts_with("Syntax error: "));
        let expansion = ShellError::ExpansionError("NOPE: unbound variable".to_string());
        assert!(expansion.to_string().starts_with("Expansion error: "));
    }
}
