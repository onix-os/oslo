# The shape of a built-in tool

oslo's plans have deferred "the built-in tools" several times without saying what one *is*. That is
the actual deferred decision, and it has to be settled before any individual tool is worth writing.
This is the shape. No tool is implemented yet.

## What problem they solve

A shell spends its life shelling out to `ls`, `df`, `ps`, `grep`. Each answers in text that the next
program has to re-parse, and the parsing is where scripts break: `df` output shifts when a mount
point is long, `ls -l` is ambiguous for a filename with spaces, `ps` columns differ per platform.

oslo has a Lua evaluator and a table type. A built-in tool answers in a **table** when Lua asks, and
in **text** when a pipe asks. Same tool, same name, two shapes.

## The rule that decides everything

**A tool is a shell builtin first and a Lua function second.** Not the reverse.

`df` typed at the prompt must behave like `df` — same output, same columns, pipeable into `awk`.
Anyone who cannot rely on that cannot use oslo as `/bin/sh`, which is the whole project. The Lua
table is what the *same code* returns when it is called from Lua instead of from a command line.

```sh
$ df -h                      # text, exactly as expected, pipeable
$ echo "=for _, m in ipairs(sh.df()) do print(m.mount, m.free) end"
```

## Registration

A tool registers once and declares both faces:

```rust
Tool {
    name: "df",
    // The argv face: writes text to stdout, answers an exit status.
    run: fn(&mut Environment, &[String]) -> Result<i32>,
    // The Lua face: answers rows.
    rows: fn(&mut Environment, &[String]) -> Result<Vec<Row>>,
}
```

`run` is defined *in terms of* `rows` for tools whose text output is a rendering of the rows — which
is most of them. That is what keeps the two faces from drifting: there is one source of facts and
one renderer, not two implementations that agree by accident.

A tool that cannot be expressed as rows (`clear`, `sleep`) has no `rows` and is simply a builtin.
Nothing is forced into a table shape it does not have.

## What a row is

A Lua table with named fields, not a positional list. Names are the point: `m.free` survives a
column order change, `m[4]` does not.

Fields carry **values, not renderings**. `size` is a byte count, not `"4.2K"`. Human forms are
offered alongside where they are cheap (`size_human`), the same way the completion dropdown already
exposes both — a config that wants to compare needs the number, one that wants to draw wants the
string, and making each config reimplement the formatting is how they end up disagreeing.

## Where the sugar lives

`sh.df()` is the Lua face. `sh` already exists as sugar over `oslo.run`, so a built-in tool slots in
beside external commands without a second namespace to learn. `sh.df()` returns rows;
`sh.df{"-h"}` passes arguments through.

The rule stays: `sh.<name>` prefers the built-in tool when there is one, and falls through to the
external program when there is not. A script that runs on a machine where oslo is not the shell
still works, because the external `df` is still there.

## Which tools, and in what order

Not all of them, and not clones. The plan's phrasing is "aiming past POSIX rather than cloning it",
which means a tool earns its place by being *better as a table* than the external one is as text.

Good candidates, in the order their table shape is most obviously worth having:

| Tool | Why a table beats text |
|---|---|
| `df` | Mount points with spaces break every `awk` invocation ever written against it |
| `ps` | Column sets differ between platforms; a table does not |
| `ls` | Filenames with newlines make the text form genuinely ambiguous |
| `stat` | Already a struct pretending to be text |
| `env` | Values containing `=` or newlines cannot be parsed back out of the text form |

Bad candidates: `echo`, `printf`, `cat`, `test`. They have no rows, and a table would be an
affectation.

## What this does not decide

* Whether tools live in the main binary or a separate crate. That is a build question, and it
  should be answered once one tool exists and its weight is measurable.
* Streaming. Every tool above answers a bounded set of rows. `ps` on a machine with 50,000
  processes is still bounded; `find /` is not, and a streaming tool needs a different shape than
  `Vec<Row>`. Deferred until a tool needs it, deliberately.
* Colour and column layout for the text face. The dropdown's layout code already solves the
  "fit N columns into a terminal" problem and is the obvious thing to reuse, but reuse should be
  proven rather than assumed.

## The first one to build

`df`. Small, bounded, its text form is genuinely hard to parse, and it exercises every part of the
shape: two faces, named fields, a value-plus-rendering pair (`size` / `size_human`), and arguments
that change the text face (`-h`) without changing the rows.
