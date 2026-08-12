# Macros

Four kinds of small named thing — an alias, an abbreviation, a function, a script — kept in one
database and managed with one subcommand, so that saving one does not mean editing a file and
re-sourcing it.

```sh
oslo macros add --alias  gs 'git status --short' --tag git
oslo macros add --abbrev gco 'git checkout'      # expanded into your line as you type
oslo macros add --func   mkcd                    # opens your editor
oslo macros add --script deploy                  # opens your editor, any language
oslo macros show                                 # the manager, on the whole screen
```

The kind is required, because four kinds and a silent default is a trap. An inline body is for the
two that fit on a line; a function and a script always open the editor.

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
   ├─ does macros.db exist at all?      no ──────────► "command not found"
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
oslo macros add --alias gs …
   │
   ├─ put into macros/macros.db          the store, for the manager
   └─ publish → macros/macros.snapshot   a flat file, for a starting shell

a new interactive shell
   │
   ├─ config.lua                          alias gs = …
   ├─ write macros/elsewhere.snapshot     ← what the *config* defined, for the manager
   └─ read macros.snapshot (3.6 µs)       the database wins, see below
```

**Interactive shells only.** This runs beside `load_config` and nowhere else, so `oslo -c` and a
script see none of it. That takes nothing away: `config.lua` is read by the same loop and by nothing
else either, so a non-interactive shell has never had aliases to expand.

### And a change reaches the terminal beside this one

Universal variables already solve this problem — *write it from anywhere, see it everywhere* — and
the mechanism is one `stat` per prompt on a file that is read only when it has moved. Macros use the
same one, deliberately:

```
per prompt:  stat macros.snapshot   ─┐
             stat session/<id>.off  ─┴─ moved? ─► rebuild:  what the config defined,
                                                            then the database on top,
                                                            minus what is off here
```

So `oslo macros add` in one terminal is live in the others before their next prompt, and a shell
where nothing changed pays two syscalls and no parse.

**The rebuild is a whole set, not a patch**, because the interesting case is *removal*: when `gs`
leaves the database the right answer is whatever your configuration said about `gs` — which may be
nothing, or may be a different alias. Composing both sources and diffing against what this shell was
last given answers that. An alias you typed at the prompt is never in that set, so it is never in the
difference, and a rebuild cannot take it away from you.

## Order: config first, database last

Three things define an alias — `alias` in a script, `oslo.alias` in `config.lua`, `oslo macros add`
— and the ordinary shell rule is that the last definition wins. That rule is applied to the sources:
the database is applied after the configuration, so **the database wins**.

That is the deliberate half. The database is the one you can change without editing a file, so
`oslo macros add --alias gco …` taking effect is what you asked for. The cost is that a stored entry
can shadow one you wrote in `config.lua` — which is why a shell writes down what its config defined,
and why the manager can show you both: **Tab** moves between `[stored]` and `[elsewhere]`.

`[elsewhere]` is aliases and abbreviations only. A function is a file on disk and a Lua function is a
name in a table; there is nothing to enumerate, so that source can never show either.

## The manager

`oslo macros show` on a terminal is a full screen, and it is the history finder's screen: the same
look, the same striping, the same search bar, the same delete confirmation. A second full-screen list
that behaved differently would be a second thing to learn.

**Alt+\\ opens it from the prompt** — beside Ctrl+\\ for the [scratch finder](scratch.md), the same
key with the other modifier for the other list of things you keep. `oslo.macros.key` moves it, and
takes any key name a config can write.

Alt rather than a Ctrl+Enter, which is the obvious choice and does not work: Ctrl+Enter sends the
same `\r` as Enter unless the terminal speaks the kitty keyboard protocol, and even then oslo
decodes it as Ctrl+M — historically the same key. Alt sends `ESC` and the character, everywhere,
with nothing to negotiate.

```
   3d   alias   gs        git status --short                    #git #system
   3d   abbrev  gco       git checkout                          #git
  12d   script  deploy    #!/usr/bin/env python3                #work
 ✗ 1h   func    mkcd      mkdir -p "$1" && cd "$1"

  ⬝⬝⬝⬝⬝⬝  >>  de▏                        [stored] @ [#work] || 1/4
```

| key | |
|---|---|
| type | filter — every column, so `script` narrows by kind and `git` by tag |
| ← → | **the tag**: all of them, then each one in use. Where the finder's scopes are |
| Tab | **the source**: `[stored]` ↔ `[elsewhere]`. Where the finder's profile is |
| Enter | **the editor**, for every kind, including an alias |
| Delete | forget it, after the same question the finder asks |
| Space | off **for this session** |
| Space ×3 | off **everywhere**, until three more turn it back on |

Enter is the one that differs from the finder, and it has to: the finder puts a line back on the
prompt because a past command is something to run again, while a macro is something you keep — and
what you want from it is to change it. Enter on an `[elsewhere]` row copies it into the database
first, because a row that came from a file is not ours to write back to. That copy is also how you
get a configured alias you can turn off.

**Off is not gone.** A macro turned off keeps its body, its tags and the day it was made; what
changes is that it stops applying. Turning off an alias that shadows a configured one uncovers the
configured one, because off means *this one does not apply* rather than *this name does nothing*. A
row that is off is drawn muted with a marker, because a list where a disabled row looks exactly like
a live one answers the wrong question.

Both switches have a spelling for something that is not a person:

```sh
oslo macros off gs             # everywhere, now and next time
oslo macros off gs --session   # this shell only, until it closes
oslo macros on  gs
```

A session's list is a file named for the session, and the session is `$OSLO_SESSION` — which a shell
exports precisely so that a child process can name the session it is part of. `oslo macros` is a
child of the shell whose macros it manages; without a name both agree on, "off for this session"
would be written down for a session nobody is running.

## Editing, and getting back out

`--func` and `--script` always open an editor, because neither fits on a command line and pretending
otherwise invites a function written as one. The editor is `$VISUAL`, then `$EDITOR`, then `nvim`,
then `vi` — the two variables first because they are what you have already told every other program
on the machine. The temporary file is named for the language it holds (`.sh`, `.lua`, `.py`, or
whatever the shebang says), because syntax highlighting is most of the reason to want a real editor.
Quitting without saving stores nothing and says `unchanged`.

The screen closes while the editor runs and comes back with the query it had. Keeping it open around
a child that also drives the terminal is how two programs end up fighting over the same termios.

Piped, or with `--plain`, there is no widget and no colour: one tab-separated row per macro, every
field included. A manager only a person can read is one you cannot script.

A database is not a dotfiles repository: an alias in `config.lua` is version-controlled, diffable and
copied to a new machine with the rest of your configuration, and one in here is none of those.
`export` and `import` are the way back out, in a format meant to be read and hand-edited:

```
alias gs #git #system
	git status --short
func mkcd off
	mkdir -p "$1" && cd "$1"
```

The header carries the kind, the name, `off` if it is turned off, and `#tag` for each tag; the body
follows, indented by one tab.

## Tags, where a profile would be

A history is per profile because it is a record of what a particular shell did, and an agent's
commands must not pollute the ranking of yours. A macro is not a record of anything, so there is
**one database for the user** and a tag is a label on a row — several of them, if you like, which is
the thing a profile could never be.

```sh
oslo macros add --alias gs 'git status --short' --tag git --tag system
```

← and → move through the tags in use, in the place the finder puts its scopes.

## Where it lives

| | |
|---|---|
| `crates/oslo-base/src/macros.rs` | the store, the four kinds, the record |
| `crates/oslo-base/src/macros/live.rs` | the two sources, the rebuild, the session list |
| `crates/oslo-runtime/src/startup/stored.rs` | applying it to a shell, at startup and per prompt |
| `crates/oslo-shell/src/exec/stored.rs` | running a function or a script, and the memfd |
| `crates/oslo-ui/src/manager.rs` | the screen: rows, keys, and what it asks the caller to do |
| `crates/oslo-runtime/src/macros.rs` | the screen over the real database, for the key and the CLI |
| `crates/oslo-runtime/src/editor.rs` | handing text to `$EDITOR` and taking it back |
| `src/cli/macros.rs` | `add`, `remove`, `show`, `off`, `on`, `export`, `import` |
| `~/.local/share/oslo/macros/` | `macros.db`, `macros.snapshot`, `elsewhere.snapshot`, `session/` |

## What it cannot do

- **`[elsewhere]` cannot show a function or a script.** There is nothing to enumerate: one is a file
  on disk, the other a name in a Lua table.
- **No aliases or abbreviations in a non-interactive shell.** `oslo -c` and a script never read
  `config.lua` either. Functions and scripts are found when you call them, so those work anywhere.
- **A configured alias cannot be edited in place** — only copied into the database and edited there.
  The file it came from is yours, and writing to it would be this deciding how your config is
  formatted.
- **No tag editing after the fact.** `--tag` on `add` sets them; changing them means adding again.
- **No new list widget, and no editor of its own.** The screen is the finder's look; the editor is
  `$VISUAL`/`$EDITOR`, with `nvim` only as a fallback.
- **No temporary file to run a script.** That is what the memfd is for.
- **No shadowing of anything on `$PATH`.**
