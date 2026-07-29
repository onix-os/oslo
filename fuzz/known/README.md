# Known findings

Bugs the fuzzer has found that are still open. One file per finding; the file *is* the reproducer,
and `known_findings_are_still_open` in `src/targets.rs` fails the day one of them stops
reproducing. Fixing a bug therefore means deleting its file and its entry here — the same ratchet
`tests/differential/expected_fail.rs` runs on the differential corpus.

Nothing in this directory is copied into the fuzzing corpus. The corpus replay in CI has to stay
clean, or a real regression would arrive as one more red among several.

---

## `lexer_unicode_whitespace_never_advances` — one 0x0b byte

`Lexer::next` returns an endless stream of empty `Word` tokens, so every caller that loops to
`Eof` spins forever, allocating as it goes.

```
$ printf 'a=(1\v2); echo ok\n' > x.sh; rush x.sh     # hangs; bash prints ok
$ printf "trap 'echo t' EXIT\vtrap - EXIT\n" > y.sh; rush y.sh   # hangs at parse time
```

Two functions disagree about what whitespace is. `Lexer::skip_whitespace`
(`src/lexer/scanner.rs:73-85`) advances past `' '`, `'\t'` and `'\r'` only, while
`scan_word_parts` (`src/lexer/quoting.rs:69`) ends a word at any `char::is_whitespace()`. A
character in the gap between the two — U+000B vertical tab, U+000C form feed, U+00A0 no-break
space, U+2028, and the rest of Unicode `White_Space` — is skipped by neither and consumed by
neither: `scan_word` returns a word with no parts, the cursor has not moved, and the next call
does the same thing forever.

The severe caller is the parser, not the lexer. `convert_words_from_str`
(`src/parser/brush_adapter/words.rs:65-72`) loops `lexer.next()` until `Eof` and pushes every
`Word` it sees, so *parsing* a script with such a character inside a word never returns and grows
a `Vec` without bound — a hang and an out-of-memory from data, on the path every script takes.
`Lexer::new` is also called on an array-literal body (`src/env/builtins/arrays.rs:24`) and an
alias body (`src/exec/simple.rs:402`), both likewise text the shell was handed rather than text a
user typed at it. A no-break space pasted out of a document or a web page is enough.

Two ways it shows up in this directory, one bug behind both:

* `fuzz_lexer` panics on the committed reproducer, because `lex_word` refuses to accept more
  tokens than the input has bytes.
* `fuzz_parse` reports a libFuzzer *timeout*, because nothing in the parse path bounds the loop.

The committed reproducer is the lexer one: a file that hangs cannot live in a directory that
`cargo test` replays.

The fix belongs in `src/lexer/`, which this directory does not own: make the two functions agree,
by having `skip_whitespace` skip every `char::is_whitespace()` other than `'\n'` (which is a
token) — and, defensively, have `scan_word` refuse to return a token when the cursor has not
moved, so the next caller to loop on `next()` cannot be wedged by whatever the next disagreement
turns out to be.

---

## `parser_unbalanced_nesting_is_exponential` — 40 `(` and a word

Parse time doubles with every unmatched `(`. Measured on a debug build, `rush -c "$(printf '(%.0s' $(seq $n))x"`:

| unmatched `(` | rush | bash |
|---|---|---|
| 10 | 0.01 s | instant syntax error |
| 20 | 0.64 s | instant syntax error |
| 25 | 15.9 s | instant syntax error |
| 30 | did not finish in 30 s | instant syntax error |

Balanced parentheses are fine at any depth the nesting guard allows: `((((…))))` at depth 40
parses in milliseconds. It is the *unmatched* opener that makes `brush_parser` — a PEG, so
backtracking — re-try an exponential number of alternatives before it can conclude the input is
malformed.

`parser::nesting::check_nesting` does not help. `MAX_INPUT_NESTING` is 100, and it was measured
against a stack overflow at 400–600 levels, not against parse time: 30 characters of nesting are
already unbounded work, and 30 is a third of what the guard permits. The header of
`src/parser/nesting.rs` says "refuses only input nested far beyond anything a person writes",
which is true and, for this failure mode, not enough.

Reachable the same way every parse is: `rush script.sh`, `eval`, `source`, a heredoc-free
`$(…)` body, or a paste into an interactive prompt. The shell does not print a syntax error, does
not exit and cannot be interrupted usefully — it sits at 100% CPU. bash rejects all of these in
under a millisecond, which is the differential test this deserves once it is fixed.

The fix belongs in `src/parser/`: either bound the parser's work (a depth limit low enough to
matter — real scripts nest single digits, per the same comment in `nesting.rs`), or reject
unbalanced openers in the pre-scan that already walks the input to count nesting. The pre-scan
knows the depth never returns to zero; it just does not currently say so.
