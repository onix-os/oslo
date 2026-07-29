//! Fuzz harness for the three rush parsers that consume text nobody vetted.
//!
//! A shell reads attacker-shaped input by definition: a script downloaded from anywhere, a
//! `$(( ))` body built from a variable, a completion candidate typed at a prompt. All three of the
//! entry points fuzzed here take a `&str` and are reachable from data:
//!
//! * [`targets::parse_script`] — `brush_adapter::parse_bash_script`, the only parser rush has.
//! * [`targets::lex_word`] — the word lexer and its token scanner.
//! * [`targets::eval_arith`] — `eval_arithmetic`, whose wrapping operators and resolve-depth
//!   guard are the thing PLAN.md R3.5 asks to keep honest after the R3.1–R3.4 rewrite.
//!
//! Everything the fuzz targets do lives in this library rather than in the `fuzz_targets/` stubs,
//! for one practical reason: a `#![no_main]` libFuzzer binary needs a nightly toolchain and a
//! sanitizer runtime, and `cargo test --lib` here needs neither. The corpus replay in
//! [`targets`] therefore runs on any machine that can build rush at all, so a missing nightly
//! costs coverage but never costs the check entirely.
//!
//! ## What the harness will not do
//!
//! Arithmetic expansion runs command substitution for real — `$(( $(rm -rf ~) ))` is a fork and
//! an exec, not a parse. [`opens_command_substitution`] drops those inputs before evaluation.
//! A fuzzer that can run commands is a fuzzer that can destroy the machine running it, and no
//! amount of extra coverage is worth that trade.

pub mod targets;

/// Longest script the parser target will look at.
///
/// The seed corpus (`tests/corpus`) averages under 200 bytes a file and its largest member is
/// well under 4 KiB, so this is not a limit real inputs hit. It exists because libFuzzer measures
/// throughput in executions per second, and a megabyte of `(((((…` buys depth coverage the
/// nesting guard already rejects at 100 levels.
pub const MAX_SCRIPT: usize = 64 * 1024;

/// Longest single word the lexer target will look at. A word is not a program; anything past this
/// is a repetition of coverage already reached.
pub const MAX_WORD: usize = 16 * 1024;

/// Longest arithmetic expression the evaluator target will look at.
pub const MAX_EXPR: usize = 4 * 1024;

/// Decode fuzzer bytes into the `&str` every rush parser wants, or `None` if the input is too big.
///
/// The decode is lossy on purpose. rush's public API is `&str`, so a strict `from_utf8` would make
/// the target return early on most mutations and spend the budget rediscovering UTF-8 rather than
/// shell syntax. Lossy decoding keeps every byte string a usable test case; the replacement
/// character it produces is itself an interesting word character.
pub fn text(data: &[u8], max: usize) -> Option<String> {
    if data.len() > max {
        return None;
    }
    Some(String::from_utf8_lossy(data).into_owned())
}

/// Does this text open a command substitution?
///
/// Both spellings fork and exec. `$((` is deliberately *not* excused: whether a given `$((` is
/// arithmetic or a command substitution wrapped in a subshell is a question about the lexer, and
/// the lexer is the code under test — the harness must not bet its safety on the answer.
pub fn opens_command_substitution(text: &str) -> bool {
    text.contains('`') || text.contains("$(")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_input_is_dropped() {
        assert!(text(&vec![b'a'; MAX_EXPR + 1], MAX_EXPR).is_none());
        assert!(text(&vec![b'a'; MAX_EXPR], MAX_EXPR).is_some());
    }

    #[test]
    fn invalid_utf8_still_produces_a_test_case() {
        // 0xff is not valid UTF-8. Dropping it would throw away most of what a mutator produces.
        let decoded = text(&[b'e', 0xff, b'c'], MAX_WORD).expect("within the size limit");
        assert!(decoded.starts_with('e') && decoded.ends_with('c'));
    }

    #[test]
    fn command_substitution_is_recognised_in_both_spellings() {
        assert!(opens_command_substitution("1 + $(id -u)"));
        assert!(opens_command_substitution("1 + `id -u`"));
        // Nested arithmetic shares the `$(` prefix and is refused with it; see the doc comment.
        assert!(opens_command_substitution("$((1 + 1))"));
        assert!(!opens_command_substitution("1 + ${x:-2} * ~3"));
    }
}
