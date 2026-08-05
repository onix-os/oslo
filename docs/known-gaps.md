# Known gaps

What oslo does not do, reproducible against the binary. All but the last are differences from bash.

Kept here rather than in the README because the list grows as the differential corpus does, and a
README that grows with it stops being read.

The behavioural entries — the ones where oslo and bash disagree about a construct — are pinned by
`tests/differential/expected_fail.rs` and `fuzz/known/`, which are two-way ratchets: the suite fails
if a listed case starts *passing*, so an entry cannot quietly go stale. The last line is not, and
cannot be: "there is no `$RANDOM`" is the absence of a feature, and there is no test that fails when
somebody adds one.

- **`for ((;;))`** is a syntax error when the separators touch — write `for (( ; ; ))`. The cause is
  upstream in brush's tokenizer, which fuses the two `;` into the `;;` that ends a `case` item.
- **Process substitution** needs `/dev/fd`, so it fails in an initramfs without it. So does bash.
- **`coproc` and `select`** are refused by name rather than half-implemented.
- **`set -e` is too eager after a short-circuited `&&` inside a compound.** `set -e; if true;
  then false && echo no; fi` ends the shell; bash, dash and busybox all carry on. The AND-OR
  exemption is correct on its own — `set -e; false && echo no` is fine — but it is lost when the
  list is the *last* command of an `if`, `for`, `while` or `{ }`, because the compound inherits
  the status and is then judged on it. Pinned by
  `tests/corpus/options_errexit_and_or_in_compound.sh`.
- **Arrays are indexed only.** `declare -A` says so rather than pretending.
- **`shopt`** switches `autocd` and `globstar`; the rest report the state oslo actually has and
  *fail* when asked for the other one — an error rather than a lie.
- **A structured tool cannot read the shell's own stdin.** `df | where …` works; `cat x.json |
  oslo -c 'from json | …'` does not — structure is assembled inside one pipeline. Use
  `oslo -c 'cat x.json | from json | …'`.
- No `/dev/tcp` or restricted mode.
