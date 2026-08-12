# An alias manager, and things that are not aliases

`oslo aliases` — a database of the small named things you accumulate, and one place to add, edit,
list and remove them. Four kinds share it: an **alias**, an **abbreviation**, a **function** and a
**script**.

**Work on `feat/aliases`**, branched from `develop`. **Done** — all six steps below are built and
`make verify` is green; what shipped is written up in
[`docs/features/aliases.md`](docs/features/aliases.md).

## What exists today, and what does not

| | where it lives now | persisted |
|---|---|---|
| `alias gco='git checkout'` | `Environment`, from `config.lua` or the `alias` builtin | **no** |
| `abbr gco 'git checkout'` | `oslo_ui::abbr`, from `config.lua` or the `abbr` builtin | **no** |
| a shell function | `~/.config/oslo/functions/NAME.sh`, autoloaded on first call | as a file |
| a script | a file on `$PATH` | as a file |

Three of the four are configuration, one is a file, and **nothing survives except by editing a
file**. There is also **no `$EDITOR` integration anywhere in oslo** — nothing shells out to an editor
today, so that is new work rather than a call to something existing.

Two pieces already exist and should be used rather than rebuilt: `oslo_base::store` (a private
`0600` key-value database per name, which is what `oslo.db` is) and `ask::filter` (a list narrowed as
you type, which is the picker `show` wants).

## The research that decides the shape: running a script that has no file

A script needs a path for the kernel to `exec`. A database row is not a path. The obvious answer is
to write a temp file, and it is the wrong one: it costs a write per run, leaves rubbish to clean up,
and puts your script in a world-readable directory where somebody can read it or race it.

**oslo is Linux-only, so it can do better.** `memfd_create(2)` makes an anonymous file that exists
only in memory, and `/proc/self/fd/N` is a path the kernel will honour — shebang and all. Tested on
this machine, not assumed:

```text
#!/bin/sh              from a memfd   → ran, exit 0
#!/usr/bin/env python3 from a memfd   → ran, exit 0
sealed with F_SEAL_WRITE              → ran, and cannot be modified after it is written
readable by another user?             → no: `/memfd:name (deleted)`, reachable only through
                                        this process's own /proc/PID/fd
```

**One wart, with a fix for the common case.** The kernel rewrites `argv` for a shebang, so a script
sees `$0` as `/proc/self/fd/3` rather than its own name. For a *shell* interpreter that is repairable,
because `sh -c` takes the name as its next argument:

```text
execv("/bin/sh", ["sh", "-c", ". /proc/self/fd/3 \"$@\"", "deploy", "alpha", "beta"])
  →  $0 is: deploy ; args: alpha beta
```

Tested and correct. For any other interpreter the name goes in `$OSLO_SCRIPT` and `$0` stays the fd
path — which is honest, and better than pretending by writing a file named after the script.

## The four kinds

```sh
oslo aliases add gco 'git checkout'          # alias: the word is replaced before parsing
oslo aliases add --abbrev gco 'git checkout' # abbreviation: expanded into the line as you type
oslo aliases add --func mkcd                 # function: opens $EDITOR, sh or lua
oslo aliases add --script deploy             # script: opens $EDITOR, any language, has a shebang
oslo aliases show                            # the list, narrowed as you type
oslo aliases remove gco
```

The distinction is not decoration. Each reaches the shell by a different route, and that is what
decides where the work goes:

| kind | how it takes effect | when it must be loaded |
|---|---|---|
| alias | word substitution before the parse | **before any line is parsed** |
| abbreviation | expanded into the buffer as you type | interactive only, on the keystroke path |
| function | a name the command search finds | on first call, like `functions/*.sh` |
| script | executed from a memfd | on first call |

## When an alias loads — measured, and settled

An alias has to be in hand before the first line is parsed, and oslo is `/bin/sh`. So the question
was whether a database can be on that path at all. **It cannot**, and it does not have to be. Both
halves are measured:

```text
oslo -c true, static musl                842 µs      (dash: 622 µs)
opening a key-value store                2.61 ms     min of 50, an existing 60-row database
reading 60 rows out of it                1.13 ms
reading 60 aliases from a flat file      3.6 µs
```

A store open is **three times a whole shell start**. Adding it to `sh -c` would make oslo five times
slower than it is and seven times slower than dash, on the exact case — a hundred short-lived shells
per `make` — that the rest of this design has been protecting.

**But the second measurement is the one that resolves it.** `config.lua` is read by
`startup::repl` and by nothing else, so a non-interactive shell **never reads it**:

```text
$ oslo -c 'gco'          # with alias gco defined in config.lua
oslo: gco: command not found
```

Config aliases are already interactive-only. So a database that loads where the config loads is not a
new restriction — it is the existing one, applied consistently. `shopt expand_aliases` stays honest:
alias *expansion* does work in every shell, and always did; what a non-interactive shell has never had
is anything to expand.

**The decision, then:**

- **Aliases and abbreviations load in the interactive loop**, beside `config.lua`. A script sees
  neither, exactly as today.
- **Through a flat snapshot, not the database.** The interactive shell already pays 2.6 ms for the
  tracking store; a second store would double that for something a `read(2)` of 3.6 µs can answer.
  `oslo aliases` writes the snapshot on every change; a missing or stale one is regenerated from the
  database, so the database stays the single source of truth and the file stays a cache that can be
  deleted at any time.

**Functions and scripts need none of this** — both are looked up after the `$PATH` search has already
failed, so they cost a database open only on a line that was going to fail anyway.

## Where it lives

```text
~/.local/share/oslo/aliases/aliases.db
```

**One store for the user, not one per profile.** History is per profile because an agent's commands
must not pollute the ranking of yours; aliases are the opposite — they are *your tooling*, and adding
one in this shell and not finding it in the next is the whole of the surprise. A profile changes what
the shell *remembers*, not what it *knows how to do*.

That path is not what `oslo_base::store` builds — it puts a database at `<data>/oslo/plugins/<name>.kv`
— so this needs its own opener beside it. Small, and better than bending the plugin one into a shape
it does not mean.

**And the trade nobody should discover later:** a database is not a dotfiles repository. Today an
alias lives in `config.lua` — version-controlled, diffable, copied to a new machine with the rest of
your configuration. In a database it is none of those. `export`/`import` is the answer, and it belongs
in the first version rather than bolted on after somebody has fifty of them.

## Aliases written in Lua and in shell still work

`alias gco='git checkout'` in a script and `oslo.alias` in `config.lua` keep working exactly as they
do now. The database is a *third* source, not a replacement, and three sources need a stated rule.

**The last definition wins, and the order is: config, then database.** That is the ordinary shell
rule — a second `alias` for a name replaces the first — applied to sources rather than to lines.
Putting the database last is the deliberate half of the choice: it is the one you can change without
editing a file, and `oslo aliases add gco …` should take effect because you just asked for it. A
config that has to be edited and re-sourced would be the more surprising winner.

The cost is that a database entry can silently shadow one you wrote in `config.lua`, so it must not be
silent: **`oslo aliases show` marks an entry that shadows a configured name**, and says what it
shadows. Discovering it in the list is fine; discovering it by wondering why your config stopped
working is not.

`oslo aliases remove` puts the configured one back on the next shell, because removing the database
row leaves nothing to overwrite it with.

## Editing

No editor integration exists, so all of this is new:

- `$VISUAL`, then `$EDITOR`, then `nvim`, then `vi`. Never a hardcoded editor.
- The row goes to a temp file with the **right extension** — `.sh`, `.lua`, or whatever the shebang
  implies — because syntax highlighting is most of why you wanted a real editor.
- Read back, stored, temp file removed. Unchanged content stores nothing.
- The terminal has to be handed over and taken back: oslo owns raw mode, and an editor that inherits
  a raw terminal misbehaves. `scratch` already does this dance for a pty and is the thing to read.

## Listing

`ask::filter` — the list narrowed as you type — rather than the history finder, which is built around
`track::history::Command` and would have to be generalised first.

**Flattened to one line each**, as asked: a function is many lines, and a list of many-line entries is
not a list. A row is `kind  name  first line`; Enter shows the real thing, and `--edit` opens it in
the editor instead. Piped, or with `--plain`, there is no widget: a page of tab-separated text.

## Order

Each step ends with `make verify` green and is its own commit.

1. **The store and the snapshot.** The database at `aliases/aliases.db`, its own opener, and the
   snapshot beside it. The measurement above is done; this is the code it decided.
2. **`oslo aliases add`/`remove`/`show`** for aliases and abbreviations only — no editor, no scripts.
   The whole surface working end to end for the two simple kinds.
3. **Loading into a shell**, by whichever route step 1's measurement chose.
4. **`--func`**, with the editor round-trip and the extension-by-kind.
5. **`--script`**, and `memfd_create` + `execveat` to run one, with the `sh -c` repair for `$0`.
6. **`export`/`import`**, so the database is not a one-way door.

## What this should not do

- **No new list widget.** `ask::filter` exists.
- **No editor of its own.** `$VISUAL`/`$EDITOR`, with nvim only as a fallback.
- **No temp file for running a script.** That is what `memfd` is for.
- **No shadowing.** A stored function or script is found *after* `$PATH`, exactly as `functions/*.sh`
  is, so nothing on disk can be quietly redefined — the rule `exec/simple/autoload.rs` already states,
  and for the reason it states.

## Decided

- **One store for the user**, at `aliases/aliases.db`. Not per profile.
- **Config and database both work**, database last, and `show` marks what it shadows.
- **Interactive only, through a snapshot.** Measured: a store open is 2.6 ms against an 842 µs shell
  start, and a flat read is 3.6 µs. A non-interactive shell never read `config.lua` either, so this
  takes nothing away.

## Settled while building

1. **A stored script answers to its name** — resolved *after* `$PATH` and after `functions/*.sh`, so
   a real `deploy` on the system still wins. Same rule as functions, and worth saying out loud
   because it is the opposite of what a dotfiles `bin/` directory does. The first question asked is
   whether `aliases.db` exists at all, so a shell with nothing stored pays a `stat(2)` on a line that
   was going to fail anyway.
2. **A name that is both a function and a script** is not resolved silently: the function answers,
   and `oslo aliases show NAME` lists every kind that name has.
3. **A redirection is applied around the call**, through the same guard a function call uses, so
   `deploy > out` fails the command rather than running it on the shell's own streams.
