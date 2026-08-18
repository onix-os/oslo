# Stream coordinates

**Status: proposal. Nothing is implemented.**

Address the text a command produced, by coordinate, from the next command — whether "next" means
the next stage of a pipeline or the next thing you type.

```
{line}                    from this command's input
{line:word}
{stream:line:word}        from a stream further back
```

## The model

**One idea: every stream of text goes on a stack, and a coordinate reads off it.**

A stream is pushed whenever something produces output — a pipeline stage finishing, or a command at
the prompt finishing. Both are the same event, so stepping back through a pipeline and stepping back
through your session are the same motion, spelled the same way.

```
$ cat hosts.txt
web-01  10.0.0.1  nginx          ← line 0
web-02  10.0.0.2  apache         ← line 1
db-01   10.0.0.9  postgres       ← line 2
          ↑
        word 1
```

Both count from zero. `{0}` is all of line 0; `{0:1}` is `10.0.0.1`.

### The source is *this command's input*

If a pipe feeds the command, that is the input. If nothing does, the input is the last stream — the
previous command's output. That is a single rule, and which case you are in is visible in the line
you are looking at:

```
$ cat hosts.txt | ssh {0:0}          # input is the pipe        → ssh web-01

$ cat hosts.txt                       # …then, separately:
$ ssh {0:0}                           # input is the last output → ssh web-01
```

### `:` divides, and how many you write says what you mean

A coordinate is one to three dimensions, separated by `:`. Which is which follows from **how many
you wrote** — nothing is marked, and nothing is ambiguous:

| written | means |
|---|---|
| `{2}` | line 2 |
| `{0:1}` | line 0, word 1 |
| `{1:0:1}` | one stream back, line 0, word 1 |

Reading right to left, the last is always the word, the one before it the line, and the one before
*that* the stream. Reaching a stream means writing all three, which is the right cost: stepping back
is the rarer thing and saying so explicitly is worth two characters.

```
cat hosts.txt | echo {*:1} | echo {1:0:0}
     │                │              │
     │                │              └─ 3 dims → one stream back, line 0, word 0 → web-01
     │                └─ 2 dims → every line, word 1 → 10.0.0.1 10.0.0.2 10.0.0.9
     └─ pushes 3 lines
```

The third stage reaches past its own input to `cat`'s. Nothing is special-cased: the same stack, one
step further down.

**Why no marker character.** `|` and `;` both looked right and both are fatal: the lexer splits on
them before any expansion runs, so `{3|0:4}` does not error — it *runs `0:4}` as a command*. Braces
do not protect a metacharacter. A third `:` needs no new character and reads as Python does.

### Ranges are `..`

Every dimension takes a range, and `*` is the shorthand for "all of it".

| | |
|---|---|
| `{2}` | line 2 |
| `{-1}` | the last line |
| `{0..2:}` | lines 0 to 2, whole |
| `{..2:1}` | up to line 2, word 1 |
| `{2..:1}` | line 2 onwards, word 1 |
| `{*:0}` | first word of every line |
| `{0:*}` | every word of line 0 |
| `{-1:-1}` | last word of the last line |
| `{0:1..3}` | line 0, words 1 to 3 |
| `{1:*:0}` | one back: first word of every line |

**One wart, and it is not fixable.** A bare `{0..2}` cannot mean "lines 0 to 2" — it is brace
expansion and already means `0 1 2` in every shell there is. Claiming it would break `echo {0..2}`,
which is not a trade worth making. So a whole-line range carries a trailing colon: `{0..2:}`. Every
other range form is free, including `{0..2:1}`, `{..2:1}` and `{0..2:1..3}`.

## Three decisions

### Many values become many arguments — not many commands

`{*:1}` hands three arguments to **one** command, exactly as `"$@"` does. It does not run the
command three times.

```
$ cat hosts.txt | ping -c1 {*:1}
    ping -c1 10.0.0.1 10.0.0.2 10.0.0.9        # one process
```

**Iteration is not missing — it already exists.** `for` plus `{*:n}` reads better than any new
keyword, and it composes with everything the shell already does:

```
$ cat hosts.txt | for h in {*:0}; do ssh $h uptime; done
```

That is why there is no `each` builtin in this proposal. The xargs shape — *run this once per line*
— is a loop, and shells have loops. Inventing a second way to iterate would be a second thing to
learn for no new power.

### One value is always exactly one argument

A substituted value is **never re-split and never globbed**. If line 0 is `my file.txt  100`, then
`{0:0}` splits on whitespace and gives `my` — but a value containing a space, a quote, a `$` or a
`*` arrives at the command as a single argument, untouched.

oslo already has the machinery: `Origin::Quoted` in `expand/sugar.rs`, which is how a marked
directory's expansion avoids being re-globbed. Placeholders use the same path. Getting this wrong is
a shell that executes filenames, so it is not a detail to defer.

Words are runs of whitespace by default. A configurable separator (for TSV and friends) is worth
having later; it is deliberately not in the first version, because the syntax for it is a design
question of its own and whitespace covers most of what people reach for.

### The stack survives across prompts

Ten streams by default, so `cat a`, `ls`, then `{1:0:0}` reaches back past `ls`. Also capped by
size, because `cat somehuge.log` must not pin memory — past the cap a stream is retained truncated,
and a coordinate that lands past the truncation reads empty rather than lying.

**Empty, not an error.** `{9:9}` on a three-line file is `""`. Input is ragged; refusing to run is
worse than a blank, and the same rule already governs `{5}` in rargs.

## What has to be built

**The hard part is capture, not syntax.** Everything else follows from having the text.

1. **Capture stdout without changing what the user sees.** oslo already does this once —
   `startup/environments/live.rs` shows a live tail while output goes to a scratch descriptor, for
   `.env.lua` loading. That is the machinery, and it is the piece with real risk: it must not
   disturb an interactive program, must not reorder stdout against stderr, and must cost nothing
   when no placeholder is in play.

2. **The stream stack** — a bounded ring of captured outputs, pushed by both pipeline stages and
   completed commands.

3. **The coordinate parser.** Every form below was checked against oslo's current brace expansion
   and globbing, unquoted, and passes through untouched: `{0}` `{0:1}` `{-1:-1}` `{*:0}` `{0:*}`
   `{0:1..3}` `{1:0:0}` `{1:1:}` `{1::}` `{1:*:0}` `{0..2:}` `{..2:1}` `{2..:1}` `{0..2:1..3}`
   `{*:*}`. The single exception is bare `{0..2}`, which is and stays brace expansion.

4. **Substitution at expansion time**, producing `Origin::Quoted` runs.

### What is deliberately not here

- **No `each`, no `xargs`, no `|>`.** Iteration is `for`. `|>` is not available anyway: it parses as
  `|` then `>`, so `echo hello |> cat` silently creates a file named `cat`.
- **No parallelism.** That is a separate feature with its own design — ordering, interleaving,
  failure accounting — and it does not belong in the addressing model.
- **No `$PREV` array, no `$ARG_PREV_n`.** Coordinates cover it: the previous command's output is
  the default source, and there is nothing to keep in sync.

## Risks worth naming before starting

**Capture is the whole feature and it touches the hot path.** If output capture is not free when
unused, every command in the shell pays for a feature most lines do not use. The gate should be that
no capture happens unless something can reference it — which argues for capturing lazily, or for
capturing only a bounded head of each stream.

**A pipeline stage's output is consumed by the next stage.** Capturing it means teeing, and a tee
between two processes is a place where back-pressure and ordering can go wrong. This is the part
most likely to need a second design pass once it is real.

**`{0:0}` reads at expansion time, before the command runs.** For a pipe, that means the upstream
must have produced at least the referenced line before the downstream command line can be built —
which is fine for `{0:0}` (one line) and impossible for `{*}` on an infinite stream. `yes | echo
{*}` cannot terminate, and should say so rather than hang.
