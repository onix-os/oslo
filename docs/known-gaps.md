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

## A structured tool reading the shell's own stdin

```console
$ printf 'a\nb\n' | oslo -c 'lines | length'
0
```

`lines`, `parse` and `from` turn bytes into rows, and they read the bytes from *the pipe they are
on*. The shell's own standard input is not that pipe, so a structured tool at the head of a
pipeline finds nothing there and answers for an empty input. Put the bytes on the pipe:

```console
$ printf 'a\nb\n' | oslo -c 'cat | lines | length'
2
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
| `for ((;;))` with touching separators | `for ((i=0;i<2;i++)); do …; done` runs, spaced or not |
| Process substitution generally | works wherever `/dev/fd` exists, which is every ordinary Linux system |
