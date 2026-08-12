# Macros: one word for four things, and a screen to manage them

`oslo macros` — the four small named things you keep, in one database, with one full-screen manager.
The word **macro** is the umbrella; the four are an **alias**, an **abbreviation**, a **function** and
a **script**. "Aliases" was the wrong name for a set that contains three things that are not aliases.

Built on `feat/aliases`, which already has the store, the snapshot, the memfd script runner and the
`add`/`remove`/`show`/`export`/`import` surface — see [`plans/PLAN_ALIASES.md`](plans/PLAN_ALIASES.md).
This plan renames that, grows the record, and replaces the picker with a real screen.

**No migration.** Nothing reads the old `aliases/` directory afterwards; it is left on disk to delete
by hand.

## What changes, in one table

| | before | after |
|---|---|---|
| subcommand | `oslo aliases` | `oslo macros` |
| directory | `<data>/oslo/aliases/` | `<data>/oslo/macros/` |
| `add` with no flag | an alias | **refused** — say which of the four |
| a record | kind, name, body | + **created**, + **tags**, + **active** |
| `show` | `ask::filter`, a dropdown | a full screen, like the history finder |
| a config-defined alias | a name in `configured.names` | a **second source** you can Tab to |
| turning one off | remove it | off for this session, or off everywhere |

## The record grows three fields

```
created   unix seconds, set once when it is first stored — the finder's first column
tags      any number, arbitrary words: `system`, `git`, `work`
active    false = off everywhere, until turned back on
```

Stored as a header line and then the body verbatim, because a body is arbitrary text and must not be
escaped through a format that then has to be unescaped exactly right:

```
1 1754870400 on system,git\n
git status --short
```

**Tags are the history finder's profiles, in the place they belong.** A profile keeps *histories*
apart because they are a record of what a shell did. A macro is not a record of anything, so one
database holds all of them and a tag is a label you put on a row — a row can carry several, which a
profile could not.

## `add` says which of the four

```sh
oslo macros add --alias  gs 'git status --short'
oslo macros add --abbrev gco 'git checkout'
oslo macros add --func   mkcd                     # editor, always
oslo macros add --script deploy                   # editor, always
oslo macros add --alias  gs 'git status' --tag git --tag system
```

A kind is now **required**: four kinds and a silent default is a trap, and `--alias` costs seven
characters. An inline body is accepted for an alias and an abbreviation and **refused** for a
function and a script — neither fits on a command line, and pretending otherwise invites a function
written as one. With no body, the editor opens.

## The screen

`oslo macros show` — the history finder's shape, its keys, and its look, because a second full-screen
list on the same machine that behaved differently would be a second thing to learn:

```
   3d   alias   gs        git status --short                    #git #system
   3d   abbrev  gco       git checkout                          #git
  12d   script  deploy    #!/usr/bin/env python3                #work
   1h   func    mkcd      mkdir -p "$1" && cd "$1"

  ⬝⬝⬝⬝⬝⬝  >>  de▏                    [stored] @ [#work] || 1/4
```

| key | |
|---|---|
| type | filter, fuzzily |
| ↑ ↓ | move |
| **← →** | **the tag**: `all`, then each tag in use. The scopes' place, exactly |
| **Tab** | **the source**: `[stored]` ↔ `[elsewhere]` |
| **Enter** | **the editor**, always — for every kind, including an alias |
| **Delete** | forget it, behind the confirmation the finder already asks |
| **Space** | off **for this session** |
| **Space ×3** | off **everywhere**, until three more turn it back on |
| Esc | leave |

A row that is off is drawn dim. `Enter` on an **elsewhere** row copies it into the database first and
then opens it, because a row that came from a file is not ours to edit — and once copied it is, which
is also how you promote a config alias into something you can turn off.

### The second source

Tab shows the aliases and abbreviations your **configuration** defined — `alias` in a bash file,
`oslo.alias` in Lua — as against the ones in the database. It cannot show functions or scripts:
those are a file on disk or a name in a Lua table, and there is nothing to enumerate.

They are known only to a shell that has run its config, so a shell writes them down at startup, the
way it already writes `configured.names` today. That file becomes the source, and grows the body and
the kind.

## Turning one off, and how a running shell finds out

Universal variables already solve this exact problem — *write it from anywhere, see it everywhere* —
and the mechanism is one `stat` per prompt on a file that is read only when it has moved. The same
mechanism, for the same cost:

```
per prompt:
  stat macros.snapshot     moved? ─┐
  stat session/<id>.off    moved? ─┴─► rebuild the alias and abbreviation tables:
                                         start from elsewhere.snapshot   (what the config said)
                                         overlay macros.snapshot          (active rows only)
                                         drop anything in session/<id>.off
```

That is what makes **off everywhere** arrive in the terminal beside this one before its next prompt,
and it is also what makes a *removal* put the config's alias back rather than leaving a hole — which
the current code cannot do, because it only ever adds.

**A function or a script needs none of this.** They are looked up when you call them, after `$PATH`
has failed, so `active` and the session list are read at that moment and cost nothing until then.

## Order

Each step ends with `make verify` green and is its own commit.

1. **Rename** `aliases` → `macros`, everywhere, with no migration.
2. **The record**: `created`, `tags`, `active`; `--tag`; a required kind flag on `add`.
3. **`elsewhere.snapshot`** written at startup, and the per-prompt rebuild keyed on both mtimes.
4. **The screen**: rows, filter, ← → over tags, Tab over sources, Enter to the editor, Delete.
5. **Space and Space ×3**, the session file, and the `active` check on the call path.
6. **Docs**: rewrite `docs/features/aliases.md` as `macros.md`, and the README.

## Decided

- **One database, tags instead of profiles.** A macro is not a record of what a shell did.
- **Enter is always the editor**, for every kind. The history finder's Enter puts a line on the
  prompt because that is what a past command is for; a macro is a thing you keep, and what you want
  from it is to change it.
- **Triple space is a gesture with a window** — three presses inside 600 ms on the same row. A single
  space is the common case and stays instant; the third press upgrades what the first two did.
- **`--plain` and a pipe still print**, with no widget. A screen that only a person can read would
  make the manager unscriptable.

## Done, and what the building added

All six steps are in, `make verify` is green, and the whole of it is written up in
[`docs/features/macros.md`](docs/features/macros.md). Three things this plan did not foresee:

1. **`$OSLO_SESSION`, exported.** "Off for this session" is a statement about the *shell*, and
   `oslo macros` is a child process with a process id of its own — so the two had no name for the
   session they shared. A shell now exports one and `track::session::id` reads it first, which also
   means a subshell and a tool report the session they are actually part of.
2. **`oslo macros off` and `on`, with `--session`.** Both switches needed a spelling for something
   that is not a person: a manager only a person can drive is one you cannot put in a script, and it
   is also what made the session switch testable.
3. **The query is matched field by field, not against the joined row.** Joining them is simpler and
   wrong: a fuzzy match has a maximum gap, so `git` against `alias gs echo gs git` has to jump ten
   characters and fails — the tag it was obviously asking for is right there and the row disappears.

## Still open

1. **Tags after the fact.** `add --tag` sets them; changing them later means adding again. A key in
   the screen, or `oslo macros tag NAME +a -b`, is the obvious next thing.
2. **Kind as a filter.** Tab is spoken for; typing `script` narrows by the kind column, since the
   kind is one of the fields the query is matched against. Good enough until it is not.
