# Known gaps

What oslo does not do, why, and what to write instead. Every entry here was re-run against the
current binary before it was written down — a gap that has quietly closed is worse than a gap,
because it sends people around a detour that is no longer there.

Two did close, and are recorded at the bottom.

## The rule these follow

**A gap says so.** Every one of these fails loudly — a syntax error naming the construct, or a
builtin refusing the option — rather than accepting the line and doing something else with it. A
shell that silently did *nearly* what you asked would be the worse outcome, because the wrong
answer arrives looking like the right one.

---

## `coproc`

```console
$ oslo -c 'coproc cat'
oslo: Syntax error: coproc is not supported yet
```

A coprocess is a two-way pipe to a background command plus an array holding its descriptors. What
it is *for* — talking to a long-running program without a temporary file — oslo does with named
pipes, which every shell has:

```sh
mkfifo /tmp/in /tmp/out
cat /tmp/in > /tmp/out &
echo hello > /tmp/in &
read reply < /tmp/out
```

## `select`

```console
$ oslo -c 'select x in a b; do echo $x; done'
oslo: Syntax error: select is not supported yet
```

`select` is a menu loop: print a numbered list, read a number, run the body with the variable set.
It is bash's, not POSIX's. In oslo the menu belongs to the interface layer, where it can be drawn
properly and arrowed through:

```sh
x=$(oslo userin choose a b c)
```

In a script that must stay portable, the loop is six lines of POSIX:

```sh
i=1; for opt in a b; do echo "$i) $opt"; i=$((i+1)); done
read -r n
```

## Associative arrays

```console
$ oslo -c 'declare -A m'
oslo: declare: -A: associative arrays are not supported
```

Indexed arrays work. A subscript that is not a number is **arithmetic**, exactly as in bash, so
`m[k]=v` writes index 0 and `m[j]=w` overwrites it — bash prints the same `k=w j=w count=1` for
that line, and oslo matching it is deliberate rather than accidental.

For a real map, `oslo.db` is a table a config owns, and a Lua table is a map:

```lua
local m = {}
m.k, m.j = "v", "w"
```

## Process substitution without `/dev/fd`

`cat <(echo hi)` works, and works by handing the reader a `/dev/fd/N` path. On a system without
`/dev/fd` — a container built without it, or a chroot missing `/proc` — there is no filename to
hand over and the construct cannot be made to work at all. Nothing to work around: a pipe or a
temporary file is the portable spelling, and it is what POSIX offers.

---

## Closed since this list was first written

| Was | Now |
|---|---|
| `for ((;;))` with touching separators | every spelling runs, empty sections and all |
| `( ( cmd ) )` read as an arithmetic command | only *adjacent* parens open one; spaced parens are nested subshells |
| A structured tool at the head of a pipeline | `printf 'a\nb\n' \| oslo -c 'lines \| length'` answers 2, not 0 |
| Process substitution generally | works wherever `/dev/fd` exists, which is every ordinary Linux system |

The first two share a shape, which is why it is worth a word: both were the tokenizer's longest
match disagreeing with the grammar. `( (` and `((` produce the same two `(` tokens, so the
arithmetic rule — tried first — matched a subshell that happened to open with another subshell, and
`( ( echo hi ) )` died evaluating `echo hi` as an expression. The fix reads the source positions the
tokens already carry and requires the two parens to touch, so a spaced pair backtracks into the
subshell rule. Guarding in the grammar rather than the tokenizer leaves `$((`, `for ((` and every
other spelling on the path they already take.

The `for` case is the same story told earlier. The
tokenizer takes the longest match, so `for ((;;))` carries **one** `;;` operator — the token that
ends a `case` item — where the grammar reads two separators. `for (( ; ; ))` parsed, which made it
look like a rule about spaces. The fix is two lines in the grammar: an arithmetic section now
*stops* at `;;`, and the condition rule accepts one as an empty condition. Nothing in the tokenizer
changed, because `;;` ends a `case` item far more often than it separates loop sections — and the
corpus case carries a `case` at the bottom to prove that still holds.
