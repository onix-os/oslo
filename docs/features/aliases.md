# The alias manager

Four kinds of small named thing — an alias, an abbreviation, a function, a script — kept in one
database and managed with one subcommand, so that saving one does not mean editing a file and
re-sourcing it.

```sh
oslo aliases add gs 'git status --short'          # an alias
oslo aliases add --abbrev gco 'git checkout'      # expanded into your line as you type
oslo aliases add --func mkcd                      # opens your editor
oslo aliases add --script deploy                  # opens your editor, any language
oslo aliases show                                 # the list, narrowed as you type
```

An alias in `config.lua` still works, and so does `alias` in a script. This is a second source, not
a replacement — see [Order](#order-config-first-database-last).

## The four kinds, and why there are four

They are not variations on one idea; they differ in *when* they act and in *what sees them*.

| kind | acts | what runs is | found |
|---|---|---|---|
| `alias` | before the line is parsed | the replacement, invisibly | at shell start |
| `abbrev` | as you type the space after it | what is in your buffer, visibly | at shell start |
| `func` | when you call the name | shell code, **in this shell** | after `$PATH` fails |
| `script` | when you call the name | its own program, in a child | after `$PATH` fails |

The pair that looks redundant is `alias` and `abbrev`, and the difference is the whole reason
[abbreviations](abbreviations.md) exist: an alias replaces the word before anything can see it, so
your history records `gs` and what actually ran is somewhere else; an abbreviation is expanded *into
the line*, so what runs is what you can read and what the history keeps.

The pair that looks like one thing is `func` and `script`. A function runs in the shell that called
it — it can `cd`, set a variable, change the shell it was called from. A script cannot: it is its own
program with its own shebang, and Python is as good an answer as `sh`.

## Running a script that has no file

A script needs a path for the kernel to `exec`, and a row in a database is not one. The obvious
answer is a temporary file, and it is the wrong one: a write per run, rubbish to clean up, and your
script sitting in a world-readable directory where it can be read or raced between the write and the
exec.

`memfd_create(2)` makes a file that exists only in memory, and `/proc/self/fd/N` is a path the kernel
will honour — shebang and all. Measured on Linux before any of this was written:

```
#!/bin/sh              from a memfd   → ran
#!/usr/bin/env python3 from a memfd   → ran
readable by another user?             → no. `/memfd:oslo:NAME (deleted)`, reachable only
                                        through this process's own /proc/PID/fd
```

So a stored script runs like this, and never touches the disk:

```
deploy alpha
   │
   ├─ $PATH search                                    → nothing
   ├─ a directory to cd into?                         → no
   ├─ functions/deploy.sh                             → no
   ▼
exec::stored::try_call
   │
   ├─ does aliases.db exist at all?      no ──────────► "command not found"
   ├─ func deploy?                       yes ─────────► run it in THIS shell
   ▼  script deploy
memory_file("deploy", body)          memfd_create(2), no MFD_CLOEXEC
   │                                 ── the child must inherit the fd; /proc/self/fd/N is
   │                                    how it reads the script
   ▼
"#!/bin/sh"  ──► sh -c '. /proc/self/fd/3 "$@"' deploy alpha
"#!…python3" ──► /proc/self/fd/3 alpha            $OSLO_SCRIPT=deploy
```

### What `$0` is

The kernel rewrites `argv` for a shebang, so a script started this way would see `$0` as
`/proc/self/fd/3` rather than its own name. For a **shell** interpreter that is repairable, because
`sh -c` takes the name as its next argument:

```
sh -c '. /proc/self/fd/3 "$@"' deploy alpha beta   →  $0 is deploy, "$@" is alpha beta
```

For any other interpreter the name goes in `$OSLO_SCRIPT` and `$0` stays the fd path. That is honest;
writing a file named after the script to make one variable prettier is not.

## No shadowing

A stored function or script is found **after** `$PATH`, exactly as `functions/*.sh` is, and for the
reason [autoloading](your-own-tools.md) states: a shell that promises a script sees POSIX behaviour
cannot have a database row quietly redefining `test`. Storing a `deploy` when the system already has
one gets you the system's. This is the opposite of what a dotfiles `bin/` directory does, and
deliberately.

It also means the database costs nothing on the ordinary path: it is opened only once a command has
already failed to resolve, and the first question asked is whether the file exists at all.

## When an alias loads — measured, and settled

Aliases and abbreviations have to be in place *before* the first line is parsed, which puts them on
the startup path, where the numbers are unforgiving:

| | |
|---|---|
| a shell start | 842 µs |
| opening the key–value store | **2.61 ms** |
| reading a flat file of the same entries | **3.6 µs** |

Opening a second store before the first prompt would cost three times the whole of startup. So the
database is not read at startup at all: every mutation republishes a **snapshot** — one flat file
beside the database — and a starting shell reads that.

```
oslo aliases add gs …
   │
   ├─ put into aliases/aliases.db          the store, for the manager
   └─ publish → aliases/aliases.snapshot   a flat file, for a starting shell

a new interactive shell
   │
   ├─ config.lua                           alias gs = …
   └─ read the snapshot (3.6 µs)           the database wins, see below
```

**Interactive shells only.** This runs beside `load_config` and nowhere else, so `oslo -c` and a
script see none of it. That takes nothing away: `config.lua` is read by the same loop and by nothing
else either, so a non-interactive shell has never had aliases to expand.

## Order: config first, database last

Three things define an alias — `alias` in a script, `oslo.alias` in `config.lua`, `oslo aliases add`
— and the ordinary shell rule is that the last definition wins. That rule is applied to the sources:
the database is applied after the configuration, so **the database wins**.

That is the deliberate half. The database is the one you can change without editing a file, so
`oslo aliases add gco …` taking effect is what you asked for. The cost is that a stored entry can
shadow one you wrote in `config.lua`, so the names the config defined are written down at startup and
`oslo aliases show` marks the row:

```
alias   gs                 git status --short  (shadows config.lua)
```

Finding that out from a list is fine. Finding it out by wondering why your config stopped working is
not.

## Editing, listing, and getting back out

`--func` and `--script` always open an editor, because neither fits on a command line and pretending
otherwise invites a function written as one. The editor is `$VISUAL`, then `$EDITOR`, then `nvim`,
then `vi` — the two variables first because they are what you have already told every other program
on the machine. The temporary file is named for the language it holds (`.sh`, `.lua`, `.py`, or
whatever the shebang says), because syntax highlighting is most of the reason to want a real editor.
Quitting without saving stores nothing and says `unchanged`.

`oslo aliases show` on a terminal is the list narrowed as you type, one row per entry — a function is
many lines and a list of many-line entries is not a list, so a row is `kind  name  first line` and
picking one shows the whole thing. `--edit` opens the one you pick. Piped or with `--plain` it is a
page of tab-separated text instead, with no widget in the way.

A database is not a dotfiles repository: an alias in `config.lua` is version-controlled, diffable and
copied to a new machine with the rest of your configuration, and one in here is none of those.
`export` and `import` are the way back out, in a format meant to be read and hand-edited:

```
alias gs
	git status --short
func mkcd
	mkdir -p "$1" && cd "$1"
```

## Where it lives

| | |
|---|---|
| `crates/oslo-base/src/aliases.rs` | the store, the four kinds, the snapshot |
| `crates/oslo-runtime/src/startup/stored.rs` | applying the snapshot to a starting shell |
| `crates/oslo-shell/src/exec/stored.rs` | running a function or a script, and the memfd |
| `src/cli/aliases.rs` | `add`, `remove`, `show`, `export`, `import` |
| `src/cli/editor.rs` | handing text to `$EDITOR` and taking it back |
| `~/.local/share/oslo/aliases/` | `aliases.db`, `aliases.snapshot`, `configured.names` |

One database for the user, not one per profile: a profile keeps *histories* apart because they are a
record of what a particular shell did, and an alias is not a record of anything.

## What it does not do

- **No new list widget.** `ask::filter` already exists and is what the picker uses.
- **No editor of its own.** `$VISUAL`/`$EDITOR`, with `nvim` only as a fallback.
- **No temporary file to run a script.** That is what the memfd is for.
- **No shadowing of anything on `$PATH`.**
- **Nothing in a non-interactive shell**, for aliases and abbreviations. Functions and scripts are
  found by name at the moment you call them, so those work anywhere.
