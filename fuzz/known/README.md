# Known findings

Bugs the fuzzer has found that are still open. One file per finding; the file *is* the reproducer,
and `known_findings_are_still_open` in `src/targets.rs` fails the day one of them stops
reproducing. Fixing a bug therefore means deleting its file and its entry here — the same ratchet
`tests/differential/expected_fail.rs` runs on the differential corpus.

Nothing in this directory is copied into the fuzzing corpus. The corpus replay in CI has to stay
clean, or a real regression would arrive as one more red among several.

---

## Open: none

Both findings this directory has held are fixed. Their reproducers were not deleted — an input
that once found a bug is the cheapest regression test there is — they moved to `fuzz/seeds/`,
where the replay runs them on every `cargo test` and the fuzzer starts from them.

### Fixed — `lexer_unicode_whitespace_never_advances` (Round 11 A1)

`Lexer::next` returned an endless stream of empty `Word` tokens, so every caller looping to `Eof`
spun forever, allocating as it went. `Lexer::skip_whitespace` advanced past `' '`, `'\t'` and
`'\r'` only, while `scan_word_parts` ended a word at any `char::is_whitespace()`. A character in
the gap — U+000B vertical tab, U+000C form feed, U+00A0 no-break space, U+2028, the rest of
Unicode `White_Space` — was skipped by neither and consumed by neither.

The severe caller was the parser, not the lexer. `convert_words_from_str` grew a `Vec` without
bound while *parsing*, so a no-break space pasted out of a web page aborted the shell on the
allocator before a single command ran.

Fixed by one shared predicate, `lexer::scanner::is_blank`, called by both functions: the shell's
separator set rather than Unicode's, which is also the set bash tokenizes on, so `echo a<NBSP>b`
now prints one word in both. Two guards keep the next disagreement from being a hang instead of
an error — `scan_word` refuses to return a token when the cursor has not moved, and
`convert_words_from_str` checks the cursor advanced on every iteration and returns a `SyntaxError`
if it did not.

Reproducer: `fuzz/seeds/fuzz_lexer/unicode_whitespace_in_a_word`.
Differential case: `tests/corpus/lexer_unicode_blanks.sh`.

### Fixed — `parser_unbalanced_nesting_is_exponential` (Round 11 A2)

Parse time doubled with every unmatched `(`: 20 openers took 0.64 s, 25 took 15.9 s, 30 did not
finish in 30 s, all at 100% CPU and none usefully interruptible. `brush_parser` is a PEG, so an
opener that never closes makes it re-try an exponential number of alternatives before it can
conclude the input is malformed. Balanced parentheses were always fine at any depth the nesting
guard allowed; it is the *unmatched* opener that costs.

`parser::nesting::check_nesting` did not help: `MAX_INPUT_NESTING` is 100 and bounds *depth*, and
25 unmatched openers is a quarter of that.

Fixed by counting what the pre-scan already knew and never reported — the openers still on its
stack when the input ends. More than `MAX_UNMATCHED_OPENERS` of them is refused. The bound is 16,
one doubling short of the 35 ms a debug build spends on sixteen, and it is reported as a
`SyntaxError` because that is what it is: bash exits 2 on every input this rejects, and so does
rush.

Reproducer: `fuzz/seeds/fuzz_parse/unmatched_openers`.
Differential case: `tests/corpus/syntax_unmatched_openers.sh`.
