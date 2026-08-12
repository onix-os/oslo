# An alias manager, and things that are not aliases

`oslo aliases` — a database of the small named things you accumulate, and one place to add, edit,
list and remove them. Four kinds share it: an **alias**, an **abbreviation**, a **function** and a
**script**.

**Work on `feat/aliases`**, branched from `develop`.

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

## The problem this creates, and it is the important one

**An alias has to be loaded before the first command, and oslo is `/bin/sh`.**

`shopt` says `expand_aliases` is permanently on — *"oslo expands aliases in every shell, not only
interactive ones"* — so a database of aliases means opening a database on **every** `sh -c` a build
spawns. oslo starts in about 3.5 ms today, and a hundred short-lived shells per `make` is exactly the
case the rest of the design has been protecting.

Three ways out. Measure before choosing:

1. **Interactive only.** Aliases from the database load when somebody is typing; a script sees none.
   This is what bash does — a non-interactive shell does not expand aliases unless asked — and it
   would mean `expand_aliases` stops being permanently on and starts answering honestly.
2. **A snapshot.** The database is the source of truth; a flat file beside it is what a shell reads,
   rewritten whenever `oslo aliases` changes something. One `read(2)` of a few hundred bytes.
3. **Pay it.** If a store open is tens of microseconds, this is an argument about nothing.

**Functions and scripts do not have this problem** — both are looked up after the `$PATH` search has
already failed, so they cost a database open only on a line that was going to fail anyway.

## Where it lives

```text
~/.local/share/oslo/aliases/<profile>/aliases.db
```

As asked — with one question worth answering first. **Should aliases be per profile?** History is,
because an agent's commands must not pollute the ranking of yours. But aliases are *your tooling*: it
is not obvious that a shell under `OSLO_PROFILE=claude` should have different ones, and a per-profile
default means adding an alias in one shell and not finding it in the next.

The alternative is one store for the user, with the profile as an optional scope. Both are one line;
they are not the same decision.

**And the trade nobody should discover later:** a database is not a dotfiles repository. Today an
alias lives in `config.lua` — version-controlled, diffable, copied to a new machine with the rest of
your configuration. In a database it is none of those. `export`/`import` is the answer, and it belongs
in the first version rather than bolted on after somebody has fifty of them.

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
not a list. A row is `kind  name  first line`; Enter opens the real thing in the editor.

## Order

Each step ends with `make verify` green and is its own commit.

1. **The store, and the measurement above.** How long does opening it take, and does an alias
   database belong on the `/bin/sh` path at all? The answer decides step 3.
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

## Open, and worth answering before step 1

1. **Per profile, or per user?** The path says per profile; the argument above says perhaps not.
2. **Do aliases load in a non-interactive shell?** Bash says no. oslo currently says yes for
   config-defined ones. A database is what makes the question cost something.
3. **Is a stored script on `$PATH`?** Typing `deploy` should probably run it — but resolved *after*
   `$PATH`, so a real `deploy` on the system still wins. Same rule as functions, and worth saying out
   loud because it is the opposite of what a dotfiles `bin/` directory does.
