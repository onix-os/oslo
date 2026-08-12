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
 │                     one for this command; else the spec's subcommands and  │
 │                     flags; else read_dir of the stem's directory           │
 │                                                                            │
 ├─ oslo.completion.sources   drop the kinds the config did not ask for ◄─────┘
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
 │                 f-b → foo-bar      /u/s/b → /usr/share/bin                │
 │ 4  Fuzzy      gap-capped subsequence, present only when fuzzy ≠ off       │
 └───────────────────────────────────────────────────────────────────────────┘
    ↓ each pass runs only when the one above it came back with nothing
    ↓ case_sensitive = true stops the walk after pass 1
```

Piece matching refuses to run unless a separator was actually typed, because without one it is a
plain prefix test wearing another name and the pass above it has already failed.

The fuzzy pass is a *gap-capped* subsequence rather than a plain one. Unbounded, `cat` matches
`create_application_target` and every other name on the machine containing those letters in order.

| preset | unmatched characters allowed between two typed letters | `gco` → `git checkout` |
|---|---|---|
| `off` | no subsequence pass at all | no |
| `tight` | 1 | no |
| `smart` (default) | 4 | yes |
| `loose` | 8 | yes |

The cap is measured between *consecutive* matched letters, never from the start of the string — a
distance from the start is not a gap, or nothing could ever match on its second word.

Because every candidate a fuzzy pass returns matched equally, that pass alone would hand back
`$PATH` order, so its results are scored. Two alignments are tried and the better kept: one that
prefers word starts, one that takes the earliest letter every time. Neither is right alone. The
boundary-seeking one finds `c`argo `r`un --`e`xample for `cre`, where a first-occurrence scan takes
the `r` buried inside `cargo` and strands the `e`; the plain one saves `rdme` against `README.md`,
where boundary-seeking jumps to the `m` after the dot and has no `e` left. The score rewards a
letter landing at a word start (+12), a letter adjacent to the last one (+8), and subtracts the
gap, how late the match started (capped at 20) and a quarter of what is left over (capped at 40).

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

Ranking is frecency: `count / (1 + ln(1 + age_in_hours))`, kept in `~/.oslo_frecency` as an
append-only log of `count<TAB>timestamp<TAB>name` lines folded on load, and rewritten as one line
per command by the next shell that loads it with 4096 lines or more in it. An append rather than a
rewrite while the shell runs, so two shells open at once both keep their uses instead of the last
one out overwriting the other. Only `command`, `builtin` and `subcommand` candidates are counted
when accepted; every command word of an accepted line is counted too.

## What makes it different

The list is ordered by what you have actually run. An alphabetical order was oslo's own first
answer, and it is the bug this replaced: `exit` offered `exitsnoop-bpfcc`, a command the user had
never run, ahead of the one they were plainly typing. Alphabetical is still available on request as
`sort = "alpha"` rather than being the only order there is.

`/u/s/b` reaching `/usr/share/bin` is the third pass of the built-in chain and needs no
configuration; what configuration exists only turns passes *off*.

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
oslo.completion.sources        = { "command", "builtin", "dir", "file" }  -- all kinds
```

`fuzzy` also takes a boolean: `true` means `smart`, `false` means `off`. A preset name nothing
answers to is reported at startup rather than ignored, because a typo that silently leaves fuzzy
matching off looks exactly like the feature not working.

`sources` names the kinds a candidate already carries — `command`, `builtin`, `alias`, `function`,
`variable`, `dir`, `file`, `flag`, `subcommand`. `directory` and `func` are accepted as spellings
of `dir` and `function`.

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

### Declaring a spec instead of computing one

```lua
oslo.completion.spec {
  command = "notes",
  desc = "notes kept in the shell",
  subcommands = {
    { name = "new",  desc = "start one" },
    { name = "list", desc = "every note",
      flags = { { "--since", desc = "only newer than" } } },
  },
  flags = { { "-v", "--verbose", desc = "say more" } },
}
```

The same shape the four built-in specs are written in, and it goes through the same code at Tab time:
subcommand matching, the walk down a nested tree, flags scoped to the subcommand you are inside, and
the description column. `subcommands` nests to any depth, so `docker compose up` is expressible.

A flag's spellings are the array part of its table — `{ "-v", "--verbose", desc = … }` — so it reads
the way the flag is written; `{ name = "--verbose" }` is accepted as well. An entry that is not a
table, or that names nothing, is skipped rather than refusing the whole spec: a generated list where
the third item came out wrong should still complete the other nine.

**A declared spec wins over a built-in one of the same name.** The four compiled in are a starting
point, not a claim to be right forever — `git` grows subcommands faster than this tree does.

Declaring is not computing, and that is the trade: a spec is data, so it cannot look at the
filesystem, run a command, or decide anything when Tab is pressed. `for_command` is still there for
that, and the two compose — a spec answers the *shape* of the command, a function answers what is on
the machine. There is no `takes = "duration"` on a flag, because nothing in oslo completes the
argument *to* a flag yet and the field would be a promise the Tab key does not keep.

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

A `for_command` hook reports no kind, so its candidates carry none — and `oslo.completion.sources`
filters on the kind. Setting `sources` therefore removes every config-supplied candidate.

Frecency is keyed on the displayed name alone, with no notion of where you were or what you were
doing. `sort = "alpha"` also discards the fuzzy pass's own ordering, since the final sort is by
name and nothing else.

Nothing here parses a command's `--help`: the descriptions and the subcommand and flag candidates
come from the spec registry, so a command nobody has written a spec for offers no arguments at all,
only paths.

A declared spec lives for the session that declared it. There is no file it is read from and nothing
writes one out, so a spec belongs in `config.lua` or in a plugin — which is where the code that knows
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
| `crates/oslo-ui/src/spec/definitions/` | the four written by hand: `git`, `cargo`, `docker`, `npm` |
| `crates/oslo-runtime/src/lua/api/spec.rs` | `oslo.completion.spec` — the Lua reader |
| `crates/oslo-ui/src/spec/frecency.rs` | `FrecencyTracker::get_score` — the formula |
| `crates/oslo-ui/src/settings/from_lua.rs` | how each `oslo.completion.*` key is read |
| `crates/oslo-runtime/src/lua/columns.rs` | the `columns` and `for_command` hooks |
| `bench/fuzzy.rs` | the measurement above |
