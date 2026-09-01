# Diagnostics

A caret under the word that was wrong.

```text
oslo: cols: nmae: no such column
   ╭─[ cols:1:6 ]
   │
 1 │ cols nmae
   │      ──┬─
   │        ╰─── no column of that name
   │
   │ Help: the columns here are: filesystem, size, used, free, capacity, mounted
───╯
```

## The one rule

**A script, a pipe and a test see exactly what they saw before this existed.**

```console
$ oslo -c 'df | cols nmae' 2>&1 | cat
oslo: cols: nmae: no such column
```

One line. No box, no colour, no caret, no help. This is not a preference, it is the whole design:
oslo is POSIX-first, and POSIX says what a shell writes to standard error. A multi-line coloured
report there would break `2>&1 | grep`, break every conformance suite, and break scripts written
before oslo existed.

So the report is the **drawn face** of an error and the one-liner is its **transport** — the same
split [`render_display` and `render_transport`](structured-pipelines.md) are two functions for — and
the thing that decides between them is `isatty(2)` on stderr.

**The one-liner is the report's first line.** Not a summary of it, not a rewording — the same bytes,
with the picture underneath. So nothing is lost when a report draws, nothing is printed twice, and
the two faces cannot drift into saying different things.

`tests/diagnostics_stay_plain.rs` holds this. Every converted site has a row in it giving the exact
stderr a pipe gets; a change to one of those bytes fails the suite whatever it looks like on a
terminal. It was written before a single site was converted.

## Turning it off, and turning it on

| | |
|---|---|
| `OSLO_DIAG=always` | draw, even into a pipe or a file |
| `OSLO_DIAG=never` | never draw; the one-liner everywhere |
| unset | draw when stderr is a terminal |
| `NO_COLOR` | the caret without the colour |

Colour is a separate question from whether to draw at all: somebody who turned colour off still
wants to be shown *where* the error is.

Both are read once, into a `OnceLock`. This is the failure path of every builtin in the shell, and
an `ioctl` per diagnostic is the same waste the structured planner refuses to spend on its own gate.

`OSLO_DIAG=always` also makes the reports testable without a pty, which is why
`tests/diagnostics_draw_a_caret.rs` is an ordinary fast test rather than forty lines of terminal
driving per case.

## What gets a caret, and what does not

An error earns a report when there is a **word to point at**: an operand that was wrong, an option
that does not exist, a column that is not there, an expression that did not compile.

```console
$ kill -s NOPE 1
oslo: kill: NOPE: invalid signal specification
   ╭─[ kill:1:9 ]
 1 │ kill -s NOPE 1
   │         ──┬─
   │           ╰─── not a signal
   │ Help: a signal is a name (TERM), a number (15), or SIG-prefixed (SIGTERM)
───╯
```

Everything else keeps its one-liner **on a terminal too**:

```console
$ cd /nope
oslo: cd: /nope: No such file or directory
```

There is nothing wrong with `/nope` as a word — the directory is simply not there — and a box around
that sentence is decoration, not information.

## A script names its file, its line and its code

When the diagnostic came from a script, the report is not the rebuilt command line — it is the
script:

```text
deploy.sh: line 5: kill: NOPE: invalid signal specification
   ╭─[ deploy.sh:5:9 ]
 5 │ kill -s NOPE 1
   │         ──┬─
   │           ╰─── not a signal
   │ Help: a signal is a name (TERM), a number (15), or SIG-prefixed (SIGTERM)
───╯
```

A real path, the file's own line number, and the code as written. That is the difference between a
caret and a compiler's diagnostic, and it is what makes the feature worth having on a two-hundred-line
script.

**Nothing new had to be plumbed.** `env::scope::record::origin` already answers `file: line N: ` for
every diagnostic in a script — it is what prints the prefix — so the file and the line were decided
long before this. All the report adds is *reading* that file, finding the word on that line, and
handing ariadne a real source. One change, in `diagnose::drawn`, upgraded every converted site at
once.

Every step may answer no, and each is ordinary: a prompt has no file, a `-c` string has no file,
`$LINENO` has not always been published, the file may have changed since it ran, and the word may
have come from an expansion and not appear in the text at all. Any of those falls back to the
rebuilt line. Scripts over a megabyte are not read; a diagnostic must not be the slowest thing in
the shell.

## Lua says where, so Lua gets the same

Every error Lua raises is `chunk:line: message`, and the caller that ran the chunk is holding the
source it ran. Those two are everything a report needs:

```text
oslo: broken.lua:3: could not index into a nil value
   ╭─[ broken.lua:3:1 ]
 3 │ print(t.field)
   │ ───────┬──────
   │        ╰──────── raised here
───╯
```

That covers a `source`d `.lua` file and `init.lua` itself — a config that fails at startup now shows
the line it failed on rather than naming the file and leaving you to count.

**The caret covers the whole line**, not a word, because that is the resolution Lua works at: it
reports a line and not a column, and inventing a column would be pointing at a guess. The indent is
skipped — it is not the mistake.

A **settings** problem is different again and is handled separately: `oslo.completion.sort =
"alphabetic"` is not a Lua error at all, so the value is looked for in the file instead. See above.

## Where the source line comes from

Three places. The script and the Lua chunk above are the first two; this is the third, and the one
that makes the whole thing affordable.

**A command's own words.** `diag::Snapshot` joins them back into one line and remembers where each
of them landed. That line is a perfectly good thing to point into, and it costs nothing upstream: no
parser learns to keep spans, no error type grows a field, no signature changes. It is the trick
[uutils took the same feature onto fifteen utilities with](https://github.com/uutils/coreutils).

The line is a *drawing* of the command rather than a transcript of it — single spaces, whatever
separated the words originally. By the time a builtin can complain, the shell has already expanded,
split and unquoted, so the text as typed no longer exists to be faithful to. What a person needs is
to see which of the words they gave is the one at fault.

**A file that really is one.** A config mistake points into the config:

```text
oslo: oslo.completion.sort: 'alphabetic' is not an order; use 'frecency' or 'alpha'
   ╭─[ ~/.config/oslo/init.lua:2:25 ]
 2 │ oslo.completion.sort = "alphabetic"
   │                         ─────┬────
   │                              ╰────── not one of the names this setting takes
───╯
```

The value is *looked for* rather than tracked. By the time a settings problem is reported the config
has already run, so there is no span left anywhere; the quoted value in the message is searched for
in the file instead. That is right whenever the config wrote it literally — nearly always, because a
config that computed the value would not have got it wrong in a way worth pointing at. Where it is
not found, nothing is drawn.

## A grouped option gets a caret on the letter

`-aZ` is one word carrying two options, and a message about `-Z` names a word that is not in the
argv at all. So the letter is found inside whichever word groups it:

```text
oslo: ulimit: -Z: invalid option
 1 │ ulimit -aZ
   │          ┬
   │          ╰── not an option here
   │ Help: ulimit: usage: ulimit [-HS] [-acdefilmnqrstuvx] [limit]
```

**The usage block moves into the report rather than under it.** Printed beneath a drawn box it reads
as a second, unrelated message; as the report's help it is the answer to the question the caret just
asked. Where nothing is drawn it is the line it has always been, in the place it has always been.

## Adding one

One call, with the same shape as the `eprintln!` it replaces:

```rust
crate::env::complain(
    args,                                              // the command's words
    spec,                                              // the one at fault
    &format!("kill: {spec}: invalid signal specification"),  // the one-liner, exactly
    "not a signal",                                    // against the caret
    Some(SIGNAL_SYNTAX),                               // what would have been right
);
```

`body` is everything after the origin, exactly as the `eprintln!` wrote it — which is what keeps the
transport face byte-identical. Three variants exist for the shapes that repeat:
`complain_with_usage` (a usage block follows), `complain_option` (the caret goes on one letter of a
cluster), and `complain_within` (the caret goes inside a word).

`complain` answers whether it drew. Most callers ignore it; the ones that do not are the ones with a
second line to print.

A site with nothing to point at keeps its `eprintln!`. That is a decision about the error, not an
omission.

## What it costs

Two crates: `ariadne` and `yansi`, which has no dependencies of its own. `unicode-width` is shared
with the drawn table, at the same major, so the caret's column arithmetic and the table's cannot
disagree about a width.

Behind the `diagnostics` cargo feature, which a release turns on and `scripts/build.sh --minimal`
leaves out. `oslo-base/src/diag_stub.rs` mirrors every signature and answers `false`, so **no call
site in the workspace carries a `cfg`** — the one conditional is in `lib.rs`, choosing between the
two files.

On a pipe the whole cost is one cached bool: `enabled()` is asked before a snapshot is built, before
the words are walked, before anything is formatted.

## What it cannot do

* **Nothing may panic.** Release builds are `panic = "abort"`, so a panic on the diagnostic path
  kills the shell *while it is already reporting an error*. Every offset goes through
  `diag::floor_boundary` before it reaches ariadne, and there is no `unwrap` on a span anywhere in
  the module.
* **A word the message rewrote cannot be found.** `kill -s term 1` reports `TERM`, which is not in
  the argv; nothing is drawn and the one-liner is what a person gets. A caret under the wrong word
  would be worse than no caret.
* **A grouped option in a word the message does not quote** falls back the same way.
* **`OSLO_DIAG=always 2>file`** writes the box into the file. That is what "always" means.

## Where it lives

| path | what is in it |
|---|---|
| `crates/oslo-base/src/diag.rs` | `enabled`, `Snapshot`, `Report`, `draw_source`, `floor_boundary` |
| `crates/oslo-base/src/diag_stub.rs` | the same signatures, drawing nothing |
| `crates/oslo-shell/src/env/diagnose.rs` | `complain` and its variants, `in_the_script`, `complain_lua`, `complain_at` |
| `crates/oslo-runtime/src/startup/config.rs` | `say`, `quoted` — a config *setting* problem into `init.lua` |
| `crates/oslo-runtime/src/startup/lua_init.rs` | a Lua *runtime* error in a sourced file or the config |
| `crates/oslo-shell/src/exec/simple/notfound.rs` | `command not found`, the commonest of them all |
| `tests/diagnostics_stay_plain.rs` | the rule: a pipe sees what it always saw |
| `tests/diagnostics_draw_a_caret.rs` | the other face, under `OSLO_DIAG=always` |
| `tests/diagnostics_point_at_the_word.rs` | the sweep: every diagnostic that names something, or a reason |
