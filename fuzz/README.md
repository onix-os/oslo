# Fuzzing oslo

Three `cargo-fuzz` targets over the parsers that read text nobody vetted:

| Target | Entry point | Why it is here |
|---|---|---|
| `fuzz_parse` | `brush_adapter::parse_bash_script` | The only parser oslo has. Every script, `eval`, `source` and alias body arrives through it. |
| `fuzz_lexer` | `Lexer` and `parse_single_word` | Quoting, `$`-forms, ANSI-C escapes. Also called on array literals and alias bodies, which are data. |
| `fuzz_arith` | `eval_arithmetic` | PLAN.md R3.5: proof that the Round 1 overflow guards and the Round 3 lexer/parser/eval split hold together. |

The harness bodies live in `src/targets.rs`, not in `fuzz_targets/`. That is what lets the same
code run two ways: under libFuzzer on nightly, and as an ordinary `cargo test` replay of the
committed corpus on stable. A machine with no nightly toolchain loses coverage, never the check.

## Running it

```sh
# Stable, no libFuzzer: replay tests/corpus and fuzz/seeds through all three targets.
cargo test --manifest-path fuzz/Cargo.toml --lib

# Nightly. Build the corpus first — it is generated, not committed.
cargo install cargo-fuzz
./fuzz/seed-corpus.sh
cargo fuzz run fuzz_arith fuzz/corpus/fuzz_arith -- -runs=0            # replay only
cargo fuzz run fuzz_arith fuzz/corpus/fuzz_arith -- -max_total_time=60 # mutate
```

`cargo fuzz` needs a rustup proxy for its `+nightly`. Under a Nix shell whose `cargo` is not one,
put the toolchain on `PATH` yourself, and `libstdc++` with it:

```sh
PATH="$(rustup which --toolchain nightly cargo | xargs dirname):$PATH" \
LD_LIBRARY_PATH="$(dirname "$(gcc -print-file-name=libstdc++.so.6)")" \
cargo fuzz run fuzz_parse fuzz/corpus/fuzz_parse -- -max_total_time=60
```

## The corpus

`seed-corpus.sh` builds `fuzz/corpus/<target>/` from material already in the repository:

* `tests/corpus/` — the 375 scripts the differential suite runs against bash. Real programs
  exercising real constructs beat random bytes by a distance no fuzzing budget closes.
* extracted `$(( … ))` and `(( … ))` bodies, for `fuzz_arith`. A whole script is not an
  expression; feeding one in only ever exercises the tokeniser's first rejection.
* `fuzz/seeds/<target>/` — hand-written inputs for shapes no corpus script contains: the two
  `i64` extremes, `MIN / -1`, unterminated quotes, a heredoc whose body looks like a command.

The generated corpus is not committed (`.gitignore`); the seeds are. CI regenerates it and caches
what a night of mutation adds.

## What the harness refuses to do

Arithmetic expansion runs command substitution for real — `$(( $(rm -rf ~) ))` is a fork and an
exec, not a parse. `opens_command_substitution` drops any input containing `` ` `` or `$(` before
`fuzz_arith` evaluates it, and a test asserts that a `$(touch …)` expression leaves no file
behind. A fuzzer that can run commands is a fuzzer that can destroy the machine running it.

`fuzz_arith` also clears the inherited process environment once, before the first `Environment`
exists, and installs a fixed set of variables. A crash that only reproduces under one developer's
exported variables is a crash nobody can act on.

## Open findings

`known/` holds reproducers for bugs the fuzzer found that are not fixed yet, one file each, with
`known/README.md` explaining what each one is. `known_findings_are_still_open` fails the day one
of them stops reproducing, so a fix is forced to delete its record — the same ratchet
`tests/differential/expected_fail.rs` runs on the differential corpus.

Today that is two bugs, both denial of service from data and both found in the first five minutes
of fuzzing: a lexer that stops advancing on a vertical tab or a no-break space, and a parser whose
running time doubles with every unmatched `(`. `fuzz_parse` and `fuzz_lexer` are therefore listed
in `FUZZ_KNOWN_OPEN` in `.github/workflows/fuzz.yml`, which downgrades their mutation step to a
warning and says why. The corpus-regression step is never downgraded: a known bug excuses the
mutation step, never the committed inputs.

A finding counts as still open if it panics *or* if it does not terminate. Recording only panics
would have dropped the more serious of these two on the floor.
