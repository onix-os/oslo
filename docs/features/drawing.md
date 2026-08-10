# Drawing, and taking over what the shell draws

Two halves of the same idea. `oslo.ui.block` is the renderer the shell uses for its own reports,
handed to a config unchanged; `on-report` is the hook that is given what the shell was about to draw
and can answer "I drew it myself". Between them a config can change how the shell looks rather than
only whether a message appears.

<!-- demo:begin -->
[![drawing demo](https://asciinema.org/a/1262736.svg)](https://asciinema.org/a/1262736)
<!-- demo:end -->

## How it works

A block is a headline and a rail of labelled rows. Nothing in `block.rs` writes to a terminal —
`Block::lines` hands the rows back and the caller emits them in one write, which is what stops a
block assembled across several statements from interleaving with a command's own output, and what
lets the whole thing be tested without a tty.

### A row is a fixed prefix and whatever is left

The width is taken **once**, at construction, so every row of one block agrees even if the terminal
is resized while it is being built. `terminal_cols` asks `TIOCGWINSZ` on stdout, then stderr, then
stdin, then `$COLUMNS`, and falls back to `FALLBACK_COLS = 80`.

```
  every row spends the same cells before its text:

      INDENT        2   "  " — so a block sits under its headline
      RAIL          1   "│" — sized with display_width, never assumed to be 1
      one space     1
      LABEL_WIDTH   7   left-padded, so values line up down the block
      one space     1   written by the row format
                   ──
      prefix_width 12   decorated;  10 with no tty, where there is no rail

      budget = columns.saturating_sub(prefix_width()).max(1)

  direnv ~/data/code/tools/rush      ◄ the headline, drawn only when non-empty
    │ changed PATH                   ◄ Overflow::Count, the default
    │ aliases _b _c _r _t _v +12     ◄ " +N": how many did not fit, and its room
                                       was kept back before the items were fitted
```

The prefix is *measured* from the constants rather than written down, so a caller that changed
`INDENT` cannot silently make every row one cell too wide.

### Three overflow policies, because a row that does not fit means three different things

| policy | what it does | what it is for |
|---|---|---|
| `Count` (default) | cut, then ` +12` | a list of names, where past the edge the count is the information and the thirteenth name is not |
| `Ellipsis` | cut, then `…` | one long value, where the front is the interesting part |
| `Wrap` | continue on the next line, rail kept, label blanked | content that has to be read rather than skimmed |

Before `Overflow` existed the choice was `Count`, hardcoded, everywhere. A directory environment is
what forced the distinction: a Nix dev shell changes thirty-five variables, and printed in full that
is four wrapped lines on every `cd`. `Ellipsis` keeps one cell back for the `…`, so the result never
exceeds the budget it was given. `Wrap` breaks on whitespace, and **a single word longer than the
whole budget is hard-broken** rather than left to wrap the terminal itself, which corrupts a redraw
rather than merely looking untidy.

### The counter is a fixed point, not a subtraction

`Count` is the one that is not obvious. Room for ` +N` has to be kept back before the items are
fitted — but the counter's width depends on how many items are hidden, and how many are hidden
depends on how much room the counter took. So `fit_items` settles instead of computing once:

```
 reserved = 0
   ┌──────────────────────────────────────────────────────────────┐
   │ (shown, hidden) = fit_within(text, budget - reserved)         │
   │   hidden == 0                      ──────────────► done, no counter
   │   counter_width(hidden) <= reserved ─────────────► done, " +N" fits
   │   otherwise: reserved = counter_width(hidden), round again ───┘
```

It terminates because `reserved` grows monotonically: two rounds, unless hiding a few more items
pushes the number to another digit. The bug this fixes was visible — items were fitted to the whole
budget and the ` +76` appended afterwards, so a row that filled its width exactly overflowed by the
counter and the terminal wrapped it, leaving `+76` alone on the next line under a row that looked
finished. `a_counted_row_never_overflows_its_width` checks every width from 24 to 99 for it.

One deliberate exception: **at least one item always goes in**, however narrow the terminal, because
a row that showed nothing and said `+35` would be worse than one that runs a few cells over.

### `on-report`

Five subsystems print a report of their own, and each used to decide what it looked like in Rust.

```
 a reporter is about to draw
   │
   ├─ report::watched()  ── nothing attached ──────────► draw the default
   │        │ something is
   │        ▼
   │   build { kind = "…", … }   ← not built unless somebody is listening: a
   │        │                      directory arrival carries thirty-five names
   │        ▼
   │   on-report handlers, in the order they attached
   │        ├─ returned nothing / nil ─────────► ask the next handler
   │        ├─ raised ─── printed to stderr ──► ask the next handler
   │        └─ returned a value ─────────────► that is the answer; stop asking
   │                 │
   │                 ▼
   │        is the answer exactly `true`?
   │            yes ──────────────────────────► oslo draws nothing
   └────────────►no ──────────────────────────► draw the default
```

A handler that raised counts as not handled: **a broken plugin must not make the shell silent**.

| kind | fields besides `kind` | fires from | shell state |
|---|---|---|---|
| `direnv` | `state` (`loaded`/`unloaded`/`blocked`/`denied`/`failed`), `owner`; `changed` and `aliases` as `{ {name, change}, … }` when loaded; `problem` when failed | the read loop, after `cd` | free |
| `slow` | `text`, `duration_ms`, `status`, `ok` | the read loop, after a command | free |
| `chain` | `segments` — `{ {text, op, ran, ms, status}, … }`, `status` absent when the link never ran | the `chain` builtin | **held** |
| `job` | `id`, `pid`, `text`, `status`, `ended` | the job reaper | **held** |
| `time` | `real_ms`, `user_ms`, `sys_ms` | `time`'s own report | **held** |

A report has to be answered *before* the default is drawn, so unlike a notifying hook it cannot be
deferred to a moment when the shell is idle. Three of the five therefore fire with `Environment`
locked: `oslo.ui.block` is fine there because it touches no shell state, but `oslo.env.set` raises
by name. That is why every field a handler could want is passed in rather than left to be looked up.
`slow` is the odd one out — by default it emits a desktop notification rather than drawing anything,
and only once a command has taken `oslo.notify.after` seconds or more.

### The rest of `oslo.ui`

| call | answers | needs a tty |
|---|---|---|
| `oslo.ui.ask(prompt, [default])` | a string; the default at end of input | no |
| `oslo.ui.select(items, [prompt])` | index, value — or item 1 with no tty | no |
| `oslo.ui.confirm("q")` or `{question, yes, no, default}` | boolean, or nil on cancel | falls back to a line |
| `oslo.ui.input{prompt, placeholder, default, password, required}` | string or nil | yes |
| `oslo.ui.choose{items, header, multi, height}` / `.filter{…}` | a string, a list when `multi`, or nil | yes |
| `oslo.ui.write{header, placeholder, default}` | multi-line text or nil | yes |
| `oslo.ui.file{start, directories, both, hidden, height}` | a path or nil | yes |
| `oslo.ui.table{rows, headers, separator, height, no_filter}` | the chosen row or nil | yes |
| `oslo.ui.pager{text, title, wrap}` | true when it was shown | yes |
| `oslo.ui.spin{title, command, quiet}` | the command's exit status | yes |
| `oslo.ui.log(msg)` or `{message, level, time, fields}` | nothing; writes to stderr | no |
| `oslo.ui.format`, `.join`, `.style` | a string, drawn nowhere | no |
| `oslo.ui.width()`, `.height()`, `.is_tty()`, `.colors()` | what the terminal is | no |

The raw-mode ones are the same code the `ui` builtin runs, so a prompt is identical whether shell or
Lua asked for it. All of them write the question to stderr and only the answer to stdout, which is
what makes `name=$(ui input)` capture the name and nothing else.

## What makes it different

A format template for one report — bash's `$TIMEFORMAT` is the familiar one — can change the shape
of that report and nothing else. oslo does not read `TIMEFORMAT` at all: it prints bash's default
shape (whole minutes, then seconds to the millisecond) and hands `real_ms`, `user_ms` and `sys_ms`
to `on-report` as numbers, and the same hook covers the other four reporters. **A template can only
rearrange what the shell already decided to say.** The `[1]+  Done` line is likewise copied from bash column for column — the state column is
27 wide because `bash -c 'sleep 1 & jobs'` starts the command in column 34 — but here that is the
default, not the only shape.

`ui`/`oslo.ui` is oslo's answer to [gum](https://github.com/charmbracelet/gum), a separate program a
script has to have installed. These are a builtin and a Lua module over one implementation, using
the terminal the shell already owns — and cancelling is a status, not an empty answer, which is the
one thing that makes `x=$(ui input) || exit` correct.

## Configuration

```lua
local b = oslo.ui.block("direnv loaded")
b:row("PATH", "/nix/store/…:/home/…/target/debug", { overflow = "ellipsis" })
b:row("added", "_b _c _r _t _v", { label_style = "green" })
b:note("read from .env.lua", { overflow = "wrap" })
b:done()                                   -- one write, to stdout
local lines = b:lines()                    -- or the rows back, drawn nowhere
```

`overflow` is `"count"`, `"ellipsis"` or `"wrap"`. **A misspelt policy is an error, not a silent
default** — `"elipsis"` raises rather than quietly cutting a value you meant to wrap.

`style` paints the text and `label_style` the label, in the vocabulary the prompt uses: a theme slot
like `prompt.git`, a colour name, an index, or a hex triple. A name the config defined itself wins
over oslo's own, so `oslo.theme.styles["direnv.added"] = "fg:green bold"` is usable as a row style.

```lua
oslo.on.on_report(function(r)              -- or oslo.on["on-report"](…)
  if r.kind == "job" and r.ended then
    oslo.ui.block(("[%s] %s -> %d"):format(r.id, r.text, r.status)):done()
    return true                            -- handled; oslo prints nothing
  end
  -- return nothing for every other kind, and oslo draws them as usual
end)
```

Return nothing, not `false`. The first handler to return a non-nil value ends the chain, so an
explicit `false` both leaves the default drawn *and* stops any handler attached after it from being
asked.

## What it cannot do

- **Add to a report.** A handler draws the whole thing or none of it; there is no way to keep the
  default line and append to it.
- **Change the shell from three of the five kinds.** `chain`, `job` and `time` fire with the
  environment locked; `oslo.env.set` raises there, loudly and by name.
- **Move a report between streams.** `b:done()` writes to stdout, but the job notice and the `time`
  report go to stderr by default, so replacing them with a block moves them.
- **Set a block's width or decoration from Lua.** `Block::width` and `Block::plain` exist in Rust;
  the binding decides decoration once, from whether stdout is a terminal, and always fits the
  terminal. There is also no `ui block` subcommand — blocks are Lua and Rust only.
- **Style a `note`, the rail, the `…` or the ` +N`.** All four are indexed 240, and none of them
  reads from `oslo.theme`. A row's own text takes one style for the label and one for the value.
- **Guarantee a row fits.** `Count` shows one item however narrow the terminal, so on a terminal too
  narrow for one item and its counter the row runs over. That is the promise, not a defect.
- **Intercept everything the shell prints.** `on-report` covers exactly the five kinds above.
  Errors, `did you mean`, the completion dropdown and the prompt itself go their own way.
- **Nest.** A headline and flat rows; no sub-block, no columns, no alignment beyond the label
  column. `oslo.ui.join` and `oslo.ui.style` are the tools for anything boxed.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-ui/src/block.rs` | `Block`, `Overflow`, `prefix_width`, `fit_items`, `cut_to`, `wrap`, `hard_break` |
| `crates/oslo-ui/src/block/tests.rs` | the three policies at a known width, and the counter-overflow regression |
| `crates/oslo-ui/src/dropdown/width.rs` | `display_width`, `terminal_cols`, `FALLBACK_COLS` |
| `crates/oslo-ui/src/report.rs` | `on-report`: `handled`, `watched`, and the `text`/`int`/`rows` field builders |
| `crates/oslo-ui/src/ask/mod.rs` | `Answer`, `Inline`, the stderr rule, the drawn caret |
| `crates/oslo-ui/src/paint.rs` | `Panel`, and why rows are reserved before they are drawn |
| `crates/oslo-runtime/src/lua/api/ui/block.rs` | `oslo.ui.block`, `overflow_of`, `style_named_field` |
| `crates/oslo-runtime/src/lua/api/ui/prompt.rs` | the raw-mode widgets from Lua |
| `crates/oslo-runtime/src/lua/api/ui/ask.rs` | `oslo.ui.ask`, `oslo.ui.select`, `on_a_line` |
| `crates/oslo-runtime/src/lua/engine/hooks.rs` | `answer_hook_with` — first non-nil answer wins |
| `crates/oslo-runtime/src/startup/environments/report.rs`, `…/live.rs` | the direnv renderer — the only in-tree code that draws through `Block` |
| `crates/oslo-runtime/src/startup/report.rs`, `…/notify.rs` | the `direnv` and `slow` payloads |
| `crates/oslo-shell/src/exec/job/report.rs`, `…/pipeline/timing.rs` | the `job` and `time` payloads, and `describe` |
| `crates/oslo-shell/src/env/builtins/chain.rs` | the `chain` payload |
| `crates/oslo-shell/src/env/builtins/ui.rs` | the `ui` builtin |
