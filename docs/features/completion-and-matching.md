# Completion and matching

What Tab offers, and how a candidate is decided to be one. The matcher is a **transform** rather
than a prefix test — it asks whether what you typed could have been an abbreviation of the
candidate, not whether the candidate begins with it — and the dropdown that shows the answers is a
table with a kind badge and a column per kind, not a column of bare names.

<!-- demo:begin -->
[![completion-and-matching demo](https://asciinema.org/a/1262734.svg)](https://asciinema.org/a/1262734)
<!-- demo:end -->

## How it works

Tab finds the word under the cursor, decides which source answers for it, filters and sorts what
came back, and either inserts the single answer or opens the menu.

```
Tab
 │
 ├─ current_word(line, pos)     → stem, quote, command_position, prior_words
 │   └─ brace_segment()           `rm /d/{alpha,be` completes `be` against `/d/`
 │
 ├─ which source answers ─────────────────────────────────────────────────────┐
 │    $foo             variables from the environment                         │
 │    command word     builtins, aliases, functions, the $PATH index          │
 │                     ↑ this source, and only this one, walks the chain      │
 │    otherwise        oslo.completion.for_command hook, if the config set    │
 │                     one for this command; else the spec — which position   │
 │                     am I in, and what did it declare; else read_dir of the │
 │                     stem's directory                                       │
 │                                                                            │
 ├─ oslo.completion.sh_sources   drop the kinds the config did not ask for ◄─────┘
 ├─ sort   frecency descending, name as tie-break  (or name alone, sort="alpha")
 ├─ dedup  on the replacement text
 │
 └─ one candidate  → inserted, and counted as a use
    many          → dropdown, 8 rows by default
```

### The escalation

Each way of matching is tried in turn and **the first one that finds anything wins the whole
list**. They are never merged. Trying them together is how a menu fills with scattered noise while
an exact match sits in it.

```
stem = "f-b"
 ┌───────────────────────────────────────────────────────────────────────────┐
 │ 1  Exact      candidate.starts_with(stem)                                 │
 │ 2  Ignoring   the same, case-folded a character at a time                 │
 │ 3  Pieces     split on / - _ . ; every typed piece prefixes its own       │
 │                 f-b → foo-bar      d-c → docker-compose                   │
 │ 4  Fuzzy      nucleo, present only when fuzzy ≠ off                       │
 └───────────────────────────────────────────────────────────────────────────┘
    ↓ each pass runs only when the one above it came back with nothing
    ↓ case_sensitive = true stops the walk after pass 1
```

Piece matching refuses to run unless a separator was actually typed, because without one it is a
plain prefix test wearing another name and the pass above it has already failed.

The fuzzy pass is [nucleo](https://github.com/helix-editor/nucleo)'s matcher — Helix's, and fzf's in
shape — reached through `Fuzzed`, which is the only thing the six widgets that filter ever name.

| preset | what it means | `gco` → `git checkout` | `cbf` → `cargo build --features` |
|---|---|---|---|
| `off` | no subsequence pass at all | no | no |
| `tight` | the letters must be together | no | no |
| `smart` (default) | fuzzy; a capital asks for a capital | yes | yes |
| `loose` | fuzzy; case ignored whatever you typed | yes | yes |

**It used to be a cap on the gap between two typed letters, and that was the thing that made it feel
broken.** At `smart` the cap was four, so `cbf` — the query a fuzzy finder exists to answer — could
not reach `cargo build --release --all-features` at all, and the search came back empty on exactly
the abbreviation somebody had in their fingers. A cap is a crude stand-in for ranking: with a real
score, a sprawling match simply scores badly and [`Quality`](#ranking) puts it in the last tier,
where the eye never reaches it. `tight` is where "the letters must be together" still lives.

Space separates *atoms*, each matched independently and in any order, so `push git` finds
`git push`. fzf's syntax comes with it: `'exact`, `^begins`, `ends$`, `!not`.

One thing is oslo's own on top of nucleo: a small penalty for what is left over, capped, so that
`cargo` beats `cargo-nextest-runner` for `cargo` instead of tying with it and falling back to
whatever order the list was built in.

### Rows

A row is a label, a kind badge, and then any number of info columns: the description first, then
whatever the kind has left to say.

| kind | second column |
|---|---|
| `dir` | entry count, `999+ items` past a thousand |
| `file` | size, `4.2K` |
| `command` | the `$PATH` directory it was found in |
| `alias` | what it expands to |

**All of that runs at render time, on the visible rows only.** `ls /usr/bin/<Tab>` offers a few
thousand candidates and shows eight; a `stat` per candidate would be thousands of syscalls per
frame while an arrow key is held, and eight is nothing. The directory entry count is capped for the
same reason — a spool directory with half a million files in it would be counted on every frame.

Ranking is frecency: `count / (1 + ln(1 + age_in_hours))`, over **the commands you have run**. The
counts come from the profile store's run table — one row per command line, carrying `runs` and
`last_at` — folded to command names the first time a score is asked for, which is the same scan the
history finder does when it opens. Accepting a completion bumps the name for the rest of the session;
the run itself is written down when the command runs, so nothing is counted twice.

Wrappers come off, so `sudo git status` ranks `git`, and every line of a command counts towards its
name — `cargo build` and `cargo test` both rank `cargo`.

**There is no frecency file.** `~/.oslo_frecency` used to hold an append-only log of
`count<TAB>time<TAB>name`, and every reason for it had gone: the same counts were already in the
profile store, it was the only store outside XDG, and it was the profile leak — directory ranking was
per profile while command ranking was not, so an agent profile kept its `cd`s out of yours and let
every command it completed into the table that ranks yours. It also counted the wrong thing:
completions accepted rather than commands run, so a command typed in full taught the ranking nothing.

## What makes it different

The list is ordered by what you have actually run. An alphabetical order was oslo's own first
answer, and it is the bug this replaced: `exit` offered `exitsnoop-bpfcc`, a command the user had
never run, ahead of the one they were plainly typing. Alphabetical is still available on request as
`sort = "alpha"` rather than being the only order there is.

`d-c` reaching `docker-compose` is the third pass of the built-in chain and needs no
configuration; what configuration exists only turns passes *off*. It runs over **command names**,
not paths — `/u/s/b` does not reach `/usr/share/bin`, for the reason given further down.

**Fuzzy matching is in the dropdown and never in the inline ghost suggestion.** The ghost is drawn
as text appended after the cursor, so it can only ever be a strict continuation of what you typed —
a suggestion that *replaces* your line cannot be shown that way without lying about what pressing
Right will do. Pressing Tab is a request for help; typing is not.

## Configuration

```lua
oslo.completion.fuzzy          = "smart"     -- off / tight / smart / loose
oslo.completion.max_rows       = 8           -- 1 to 40
oslo.completion.descriptions   = true        -- the description column
oslo.completion.show_kind      = true        -- the kind badge
oslo.completion.case_sensitive = false       -- true stops the chain after its first pass
oslo.completion.sort           = "frecency"  -- or "alpha"
oslo.completion.sh_sources     = { "command", "builtin", "dir", "file" }  -- shell prompt
oslo.completion.lua_sources    = { "function", "field", "keyword" }       -- Lua prompt
```

`fuzzy` also takes a boolean: `true` means `smart`, `false` means `off`. A preset name nothing
answers to is reported at startup rather than ignored, because a typo that silently leaves fuzzy
matching off looks exactly like the feature not working.

**The source list is per language**, and there is no combined `sources`. The kinds are not the same
on the two sides — a shell prompt completes commands and paths, a Lua prompt completes the names in
scope — so one list could only ever be right for one of them.

`sh_sources` names the kinds a shell candidate already carries — `command`, `builtin`, `alias`,
`function`, `variable`, `dir`, `file`, `flag`, `subcommand`. `directory` and `func` are accepted as
spellings of `dir` and `function`.

Two hooks replace parts of it from Lua:

```lua
oslo.completion.columns = function(c)
  if c.kind == 'file' then
    return { c.description, c.size_human, c.age, c.mode_human }
  end
end

oslo.completion.for_command = {
  git = function(argv, current)
    if #argv == 1 then return { "add", "commit", "push", "status" } end
  end,
}
```

`columns` returning nothing falls back to the built-in columns for that candidate, so a config can
answer for the one kind it cares about. `for_command` *replaces* oslo's own candidates for that
command rather than adding to them.

### Adding candidates of your own

```lua
oslo.completion.provider {
  name = "tldr",
  kind = "example",        -- the badge, and the name `oslo.completion.sh_sources` filters on
  when = "git",            -- this command only; omit and it answers for every command
  score_offset = 20,       -- a nudge in the ranking, not a position above it
  max_items = 10,
  answer = function(ctx)   -- ctx = { command, words, current, arg, cwd }
    return { { display = "commit --amend", desc = "change the last commit" } }
  end,
}
```

**It adds; `for_command` replaces.** `oslo.completion.for_command.git` means *I own git* and oslo's
own candidates for it are dropped — the right tool when you are rewriting a command's completions,
and the wrong one for tldr, which wants three examples *beside* the subcommands oslo already knows.
So a provider's offers are merged into the list before the kind filter and before the sort, and they
compete in the same ranking rather than being stapled to one end.

Which is why a provider has the two things `for_command` never had:

- **a kind**, so `oslo.completion.sh_sources` can name it and the badge column can show it. A
  `for_command` candidate reports none at all, which is why setting `sources` silently removes every
  config-supplied candidate. A provider that declares no `kind` is badged with its own name.
- **a score offset**, because merging means competing. It is added to the frecency score in the
  existing sort — blink.cmp's `score_offset` rather than a priority that overrules everything, so a
  command you run constantly still beats a suggestion you have never taken.

A provider takes the same guards the ghost's does — `min_chars` and an `enabled` predicate — and a
list of plain strings is accepted where there is nothing to say about each one:
`return { "one", "two" }`. `examples/plugins/tldr` is the worked example. Only offers that continue the word being typed are shown, `max_items`
bounds what one provider can contribute so it cannot flood the menu, and a provider that raises loses
its own candidates and nothing else. `oslo.completion.providers()` lists what is registered.

### Declaring a spec instead of computing one

A **spec** describes a command: its subcommands, its flags, and — the part that makes the difference
between `git checkout <Tab>` offering filenames and offering branches — what each *argument position*
completes to.

```lua
oslo.completion.spec {
  command = "deploy",
  desc    = "put it somewhere",
  aliases = { "dep" },
  persistent = { ["--config="] = "which config" },   -- inherited by every subcommand
  flags = {
    { "-v", "--verbose", desc = "say more" },
    { "--env=", desc = "which environment", values = { "dev", "staging\tthe shared one" } },
  },
  positional     = { { "build", "clean" }, { "$files([.yaml])" } },
  positional_any = { "$files" },
  subcommands = {
    { name = "build", aliases = { "b" }, desc = "make it",
      positional = { function(ctx) return sh.lines("git branch --format='%(refname:short)'") end } },
  },
}
```

`subcommands` nests to any depth, so `docker compose up` is expressible. A flag's spellings are the
array part of its table — `{ "-v", "--verbose", desc = … }` — so it reads the way the flag is
written; `{ name = "--verbose" }` and the key form below are accepted as well. An entry that is not
a table, or that names nothing, is skipped rather than refusing the whole spec: a generated list
where the third item came out wrong should still complete the other nine.

**A declared spec wins over a built-in one of the same name.** The four compiled in are a starting
point, not a claim to be right forever — `git` grows subcommands faster than this tree does.

### The shape is carapace's

Field for field, so that a spec written in Lua, a spec generated from a clap program by
[carapace-spec-clap], and a spec read from a `.yaml` file are the same object. The flag modifiers
are carapace's too, and they are not decoration — `=` is what tells the walk that the next word
belongs to the flag rather than being the command's first argument:

| suffix | means |
|---|---|
| `--file=` | takes a value; the next word is it |
| `--optarg?` | takes an optional one, only when written `--optarg=x` |
| `--verbose*` | may be repeated |
| `--internal&` | real, and never offered |
| `--out!` | required |

They can be written as the key, which is how a spec file writes them and how the terse case reads
best — `flags = { ["-f, --file="] = "which file" }` — or spelled out as `takes = "value"`,
`hidden = true`, `nargs = 2`, `default = "/tmp/out"`.

`parsing` says what happens to flags after the first argument: `interspersed` (the default),
`non-interspersed` — the first argument ends flag parsing, as `ssh host -l` does — or `disabled`,
where nothing is a flag because the words belong to whatever runs next.

`dash` and `dash_any` are the positions after a bare `--`.

[carapace-spec-clap]: https://github.com/carapace-sh/carapace-spec-clap

### What a position offers

A position is a list of **values**, **macros** and **modifiers**, or a Lua function.

```lua
positional = {
  { "dev", "staging\tthe shared one", "$files([.yaml])", "$tag(environment)" },
  function(ctx) return { { value = "main", desc = "the trunk", tag = "branch" } } end,
}
```

A value is `text`, `text\tdescription`, or carapace's `text\tdescription\tstyle` — the style is
read and dropped, because oslo paints the dropdown from its own theme and has a better column for
what carapace uses colour to say. `$tag(…)` fills that column: it becomes the kind badge, which is
how the menu tells a branch from a file.

| macro | |
|---|---|
| `$files([.go, go.mod])` | files, optionally filtered by suffix |
| `$directories` | directories only |
| `$executables` | things that run |
| `$(git branch)` | run it here and read what it printed, one offer per line |
| `$bash(…)`, `$zsh(…)`, `$fish(…)`, `$nu(…)`, … | run it in that shell, if it is installed |

**`$files` is not a second path completer.** It asks for oslo's own — the one with the tildes, the
globs, the quoting, the trailing slash, the size column and the directory entry count — filtered
the way the position said. That is most of the reason to read these specs in a shell rather than in
a completion binary.

**And `$(…)` does not fork a `sh`.** carapace has to: it is a separate program completing for a
shell somewhere else. oslo *is* the shell, so the command goes down the same command-substitution
path a `$(…)` on a real line goes down — one fork, no `sh` on `$PATH` required. It is the same trick
[argc.md](argc.md) already plays.

Modifiers change what the entries before them produced, and one behind a ` ||| ` changes only the
entry it is attached to:

| modifier | |
|---|---|
| `$filter([a, b])`, `$retain([a])`, `$filterargs` | drop or keep; `filterargs` drops what the line already has |
| `$list(,)`, `$uniquelist(,)`, `$multiparts([/])` | the word is delimited and only its last piece is being completed |
| `$prefix(file://)`, `$suffix(juice)` | decorate what is inserted |
| `$tag(branch)` | the kind badge |
| `$chdir($gitworktree)` | look somewhere else — a path, or `$gitdir`, `$parent([Cargo.toml])`, `$tempdir`, `$userhomedir`, `$xdgconfighome`, `$xdgcachehome`, `$nixprofile` |

`$style`, `$usage`, `$suppress`, `$nospace`, `$noprefix`, `$shift`, `$split` and `$splitp` are read
so that a spec using one is not mistaken for a spec naming a macro that does not exist, and then
honoured by nothing: the dropdown has no elvish styles, no usage line, and appends no space there is
to suppress.

`${…}` inside a value is the line so far — `${C_VALUE}`, `${C_ARG0}`, `${C_FLAG_SUFFIX}` — with
`:-default`, `:+alt` and `//pat/rep` available, which is how the documented
`$files([${C_FLAG_SUFFIX//,/, }])` turns one flag's comma list into another macro's argument. **A
bare `$name` is always a macro, never a variable**; a substituter that treated `$files` as an unset
variable would quietly empty every spec in the world.

A **function** position is oslo's own. The string macros exist because YAML has no functions, and a
config is written in a language that does — so `positional = { fn }` is a first-class form, not an
escape hatch. It is handed `{ value, args, words, flags, dir }` and answers with strings or with
`{ value =, desc =, tag = }` tables.

### Spec files

With the `spec` feature built in, a `.yaml` file is found by the name of the command being
completed:

```
$OSLO_SPECS                      a colon list, for a project that carries its own
~/.config/oslo/specs/mycmd.yaml
~/.config/carapace/specs/mycmd.yaml
```

```yaml
name: mycmd
flags:
  --optarg?: optarg flag
  -v=: flag with value
completion:
  flag:
    optarg: ["one", "two\twith description"]
    v: ["$files"]
commands:
  - name: sub
    completion:
      positional:
        - ["$list(,)", "1", "2", "3"]
        - ["$directories"]
```

**Found by name, not read at startup.** A directory of specs is a directory of files nobody has
typed the name of yet, and reading all of them to start a shell is the cost `carapace` pays by
generating a script per shell. A spec is looked for the first time its command is completed, and the
answer — *including* "there is none" — is remembered. A machine with no specs pays one `stat` per
new command name and nothing after it.

The filename decides which command the file answers for; a `name:` that disagrees with it would
otherwise leave the file unreachable, which reads as a bug in the completion rather than a typo in
the spec. A file that does not parse is reported and skipped, costing itself and not the directory.

The YAML is a deliberate subset — block mappings and sequences, flow `[…]` and `{…}`, the three
quotings, `|` and `>` blocks, `#` comments. Anchors, aliases, tags and a second document are an
error naming the line rather than a guess: a general YAML parser is a large dependency in a binary
that measures itself in kilobytes, and a *partial* one that quietly mis-reads what it does not know
is worse than either.

`run:` is **not read**. A spec file here describes a command; it does not become one — that is a
second feature, and carapace-spec ships a separate binary for it. `exclusiveflags`, `group`,
`documentation` and `examples` are read past without complaint, because real spec files have them
and a reader that stopped at one would read almost nothing. `$spec(other.yaml)` is not read yet.

### Declaring is not always computing

A spec is still mostly *data*, and that is the trade. `for_command` is there for the rest, and the
two compose — a spec answers the shape of the command, a function answers what is on the machine.
Where a position needs to run something, `$(…)` and a Lua function are the two ways to say so, and
both live inside the spec rather than replacing it.

Until this, a config's only route was `for_command`, and the reason was one word: `CommandSpec` held
`&'static str`, which a spec built at runtime cannot be stored in at all.

## Measurements

From `cargo bench --bench fuzzy` on this machine — one short pattern (`gco`, `smart`) scored
against 3,300 candidates, which is roughly what a `$PATH` holds, averaged over 50 rounds:

| | per Tab press |
|---|---|
| folding the typed pattern per candidate | 268 µs |
| folding it once for the batch (`Fuzzed`) | 208 µs |

The 60 µs is 22 per cent of the pass, and it is entirely allocation: the scoring itself was never
the cost. The candidate still has to be folded per call, because it is different every time.

**What it cost to make specs own their strings.** `bench/spec_tab.py` presses Tab on `git comm` in a
pty forty times and reports the fastest — the deepest walk of this data the shell does, since `git`
carries the largest spec. Five runs each side, before and after:

| | fastest Tab | binary |
|---|---|---|
| `&'static str` | 0.27 ms | 7,077,888 |
| `String`, plus the `Rc` lookup | 0.27 ms | 7,109,968 |

Nothing visible, which is what was expected and not what was assumed: the walk is a `HashMap` lookup
and a handful of prefix tests against a tree of a few hundred entries, and it is drawn on a terminal
either way. The 31 KB is the Lua reader and the second registry, not the string change. Individual
runs land bimodally at either ~0.28 ms or ~0.50 ms on this machine, on both sides — which is why the
number quoted is the minimum and not the median.

## What it cannot do

The matcher chain runs for **command names only**. Filenames, spec subcommands and flags are
matched by a plain prefix test (case-folded unless `case_sensitive` is on), so `f-b` finds the
command `foo-bar` but not the file `foo-bar.txt`, and no amount of `fuzzy` changes that.

The dropdown highlights only a genuine prefix of the label, so a candidate found by the piece or
fuzzy pass arrives with nothing marked — the row does not show *why* it is a match. The full-screen
finder does mark scattered positions; the dropdown does not.

`fuzzy = "off"` is shared with the history finder and the list widgets, and there the scorer is the
only filter rather than the last pass of a chain. Turning it off leaves those lists empty as soon
as a query is typed. The `--exact` flag on the list builtins sets the same thing.

A `for_command` hook reports no kind, so its candidates carry none — and `oslo.completion.sh_sources`
filters on the kind. Setting `sources` therefore removes every config-supplied candidate.

Frecency is keyed on the displayed name alone, with no notion of where you were or what you were
doing. `sort = "alpha"` also discards the fuzzy pass's own ordering, since the final sort is by
name and nothing else.

Nothing here parses a command's `--help`: the descriptions and the subcommand and flag candidates
come from the spec registry, so a command nobody has written a spec for offers no arguments at all,
only paths.

A declared spec lives for the session that declared it. There is no file it is read from and nothing
writes one out, so a spec belongs in `init.lua` or in a plugin — which is where the code that knows
the command's shape already is.

## Where it lives

| path | what is in it |
|---|---|
| `crates/oslo-ui/src/matching.rs` | `Match`, `matchers`, `Fuzzy`, `Fuzzed`, `fuzzy_score`, `positions` |
| `crates/oslo-ui/src/matching/quality.rs` | `Quality` — the coarse kind of match, used by the finder |
| `crates/oslo-ui/src/completion.rs` | `OsloHelper::candidates`, the source split, `rank_by_fuzz` |
| `crates/oslo-ui/src/dropdown/mod.rs` | `CompletionCandidate`, `badge`, `DropdownMenu::select_interactive` |
| `crates/oslo-ui/src/dropdown/columns.rs` | `facts_for`, `builtin_columns`, `columns_for_rows` |
| `crates/oslo-ui/src/dropdown/layout.rs` | `compute_layout` — the order width is given up in |
| `crates/oslo-ui/src/dropdown/render.rs` | `render_vertical_dropdown`, `DEFAULT_ROWS`, `CEILING_ROWS` |
| `crates/oslo-ui/src/frecency_store.rs` | `FrecencyStore` — the log, the compaction |
| `crates/oslo-ui/src/spec/mod.rs` | `CommandSpec`, `SubcommandSpec`, `OptionSpec`, `SpecRegistry` |
| `crates/oslo-ui/src/spec/custom.rs` | the specs a config or a plugin declared |
| `crates/oslo-ui/src/completion/provider.rs` | the candidate providers, their kinds and offsets |
| `crates/oslo-ui/src/completion/paths.rs` | `path_candidates` — the one builder that reads the disk |
| `crates/oslo-runtime/src/lua/api/complete.rs` | `oslo.completion.provider` — the Lua reader |
| `crates/oslo-ui/src/spec/definitions/` | the four written by hand: `git`, `cargo`, `docker`, `npm` |
| `crates/oslo-runtime/src/lua/api/spec.rs` | `oslo.completion.spec` — the Lua reader |
| `crates/oslo-ui/src/spec/frecency.rs` | `FrecencyTracker::get_score` — the formula |
| `crates/oslo-ui/src/settings/from_lua.rs` | how each `oslo.completion.*` key is read |
| `crates/oslo-runtime/src/lua/columns.rs` | the `columns` and `for_command` hooks |
| `bench/fuzzy.rs` | the measurement above |
