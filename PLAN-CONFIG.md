# PLAN-CONFIG.md — the config file, and the interactive surface it drives

Every decision in **Decided** was made by the maintainer, in conversation. Everything in **Open**
is genuinely undecided and is not to be settled here. Findings are recorded with the evidence that
produced them, so a future reader can tell what was measured from what was assumed.

Source material: the IRIS source (`versenilvis/IRIS`, Go, 38k lines — read, not skimmed), fish's
interactive documentation, and an audit of oslo's own `src/interactive/`.

---

## What oslo already has

Better than expected, and it changes the shape of the work.

**oslo owns its dropdown.** `src/interactive/dropdown/` is 1,098 lines across `mod`, `layout`,
`render` and `width`, with cell-accurate measurement, `TIOCGWINSZ`, clamped column widths and its
own raw-mode selection loop. This is *not* rustyline's pager. Nothing about IRIS-style rendering is
blocked by the line editor.

**The completion kind already exists and is already populated.** `CompletionCandidate` is
`{display, replacement, description, kind}`, and `kind` is set to `command`, `builtin`, `variable`,
`dir` or `file` at every site in `completion.rs`. It is used for frecency and then **thrown away at
render time** — `render.rs` draws a marker, a fixed cyan glyph and the display text, and never
looks at `kind`. The badge is a rendering change, not a data change.

| Piece | State |
|---|---|
| `dropdown/` | Owns layout and drawing; cell-accurate; no badges, no match highlight |
| `completion.rs` | Sources: commands, builtins, variables, files, dirs. `kind` set, `description` mostly `None` |
| `highlight.rs` | **6** token types: `Command, Flag, String, Variable, Operator, Plain` |
| `hinting.rs` | Ghost text from the command index only — not history, not paths |
| `prompt.rs` | 63 lines, hardcoded ANSI, git branch, `❯`. No right prompt |
| `command_index.rs` | `$PATH` cached on `$PATH` + directory mtime |
| `frecency_store.rs` | Ranks candidates by use |
| `words.rs` | Word boundaries, quoting, command position — the hard part, already done |
| `syntax.rs` | Three-way complete/incomplete/error via the real parser |

**Colours are hardcoded ANSI literals** in `render.rs` (`\x1b[38;5;240m`, `\x1b[48;5;62m`),
`prompt.rs` (`\x1b[1;34m`) and `mod.rs`. Nothing is configurable.

**rustyline has no right-prompt support.** Confirmed by reading the crate: no `right_prompt`, no
`RightPrompt`, nothing. The note in `prompt.rs` explaining why the old one was deleted is accurate.

---

## What IRIS does, precisely

Its row is built in `integration/overlay.go`:

```
│ ▶  <icon>  <title>  [ alias ] <description>              │
     ↑        ↑typed prefix in   ↑coloured pill, colours
     Nerd      Match colour       inverted when selected
     Font      + bold
```

Two things are worth taking and one is not.

**Worth taking — the theme is named roles, not per-element literals.** Eleven of them:
`Border, Accent, Muted, Text, TextSel, Match, Desc, DescSel, SelBg, ScrollInfo, GhostText`. Every
element maps onto one. Adding a row type costs no new colour.

**Worth taking — the kind is a pill, not a word.** ` alias `, ` history `, ` system ` drawn with
their own background, inverted when the row is selected. It reads at a glance where a plain word in
the description column does not.

**Not taking — the icons.** `integration/icons.go` is a 139-line hand-maintained map from command
name to Nerd Font glyph (`git → 󰊢`, `cargo → `), falling back to `❯`. It needs a font oslo cannot
assume, and it is a list somebody has to keep feeding. Decided against; see below.

---

## What fish does

Its configuration surface *is* its colour variables — 24 `fish_color_*` and 13
`fish_pager_color_*`. The list is worth having in full, because it is the specification of how deep
syntax highlighting should go:

`normal, command, builtin, function, keyword, quote, redirection, end, error, param, valid_path,
option, comment, selection, operator, escape, autosuggestion, cwd, cwd_root, user, host,
host_remote, status, cancel, search_match, history_current`

Three of those are the interesting ones:

* **`error`** — fish highlights a command that does not exist, *as you type it*, in red. So does a
  bad redirection and a mismatched paren.
* **`valid_path`** — a parameter that names a file which actually exists is coloured differently
  from one that does not. This is the single most useful thing fish does and oslo has nothing
  like it.
* **`builtin` / `function` / `keyword` fall back to `command`** when unset, so a minimal theme
  stays small.

Autosuggestions come from three sources in order: **history, completions, file paths**. oslo does
only the middle one.

---

## Decided

| Question | Decision |
|---|---|
| Config file | **`~/.oslorc` becomes Lua, full stop.** The shell-syntax rc goes away. One config file, one language. |
| Config location | `~/.oslorc`, and `$XDG_CONFIG_HOME/oslo/config` (`config.lua` accepted). |
| Dropdown | **Badges and match highlighting, no icons.** A Nerd Font is not something a distro's `/bin/sh` can assume, and the icon map is a list somebody has to maintain for ever. |
| Colours | **A Lua theme table.** `oslo.theme = { … }` — structured, nestable, `require`-able as a module, one place to look. Not fish's flat shell variables. |
| Scope, this round | Syntax highlighting to fish's depth; autosuggestions from history and paths; prompt as a Lua function including a right prompt. |
| Out of scope | **Abbreviations.** Not chosen. |

### What "`~/.oslorc` becomes Lua" costs, stated plainly

`~/.oslorc` is *shell* syntax today (`src/startup/rc.rs`), sourced through the ordinary `source`
builtin so an alias or `PS1=` in it behaves exactly as typed. That file's meaning changes. Anyone
with an existing one gets a Lua parse error on their next login, which is the loudest possible
failure and probably the right one — but it needs a diagnostic that says *why*, not
`syntax error near 'alias'`.

Three consequences that are **not** decided and are listed in Open below: what happens to POSIX's
`$ENV`, what happens to the existing `~/.config/oslo/init.lua`, and whether a non-interactive
shell reads the file at all.

---

## The config surface

"Many many things to be set." Written as the file a user actually writes.

```lua
-- ~/.oslorc

--------------------------------------------------------------------- theme
-- Named roles, IRIS-style: everything maps onto these, so adding an element
-- costs no new colour. A string is a foreground; a table adds attributes.
oslo.theme = {
  -- the dropdown
  pager = {
    border   = "#6d6a7f",
    text     = "#edecee",  text_sel = "#ffffff",
    sel_bg   = "#3d375e",
    match    = { fg = "#61ffca", bold = true },   -- the typed prefix
    desc     = "#9692a8",  desc_sel = "#edecee",
    scroll   = "#a277ff",
    -- one entry per completion kind; the pill takes its colours from here
    kind = {
      command  = { fg = "#110f18", bg = "#a277ff" },
      builtin  = { fg = "#110f18", bg = "#61ffca" },
      file     = { fg = "#edecee", bg = "#2a2342" },
      dir      = { fg = "#110f18", bg = "#82e2ff" },
      variable = { fg = "#110f18", bg = "#ffca85" },
      history  = { fg = "#110f18", bg = "#61ffca" },
      alias    = { fg = "#110f18", bg = "#a277ff" },
    },
  },

  -- the line as you type it, to fish's depth
  syntax = {
    command    = "green",              -- resolves on $PATH
    builtin    = { fg = "green", bold = true },
    ["function"] = "green",
    keyword    = "magenta",            -- if / then / for
    error      = { fg = "red", underline = true },   -- command does not exist
    param      = "normal",
    valid_path = { underline = true }, -- the parameter names a real file
    option     = "cyan",               -- -x, --long
    quote      = "yellow",
    escape     = "magenta",            -- \n, \x41
    operator   = "cyan",               -- * ~ and expansion operators
    redirection = "blue",
    ["end"]    = "white",              -- ; &
    comment    = "brightblack",
    variable   = "blue",
    autosuggestion = "brightblack",    -- the ghost text
    match_bracket  = { bold = true },
  },
}

--------------------------------------------------------------------- prompt
oslo.prompt.left = function()
  local branch = oslo.git.branch()                    -- nil outside a repo
  return oslo.style(oslo.path.shorten(oslo.fs.cwd()), "blue")
      .. (branch and oslo.style(" (" .. branch .. ")", "green") or "")
      .. (oslo.exit_code() == 0 and " ❯ " or oslo.style(" ❯ ", "red"))
end

oslo.prompt.right = function()
  return oslo.style(os.date("%H:%M"), "brightblack")
end

oslo.prompt.continuation = "… "            -- what PS2 was

--------------------------------------------------------------------- completion
oslo.completion = {
  sources    = { "builtin", "command", "file", "dir", "variable", "history" },
  max_rows   = 15,
  descriptions = true,
  show_kind  = true,                       -- the pill
  case_sensitive = false,
  sort       = "frecency",                 -- or "alpha"
}

-- what each row shows after the label; nil falls back to the built-in columns for that kind
oslo.completion.columns = function(c)
  if c.kind == "file" then return { c.description, c.size_human, c.age, c.mode_human } end
end

-- a completion of your own, for one command
oslo.completion.for_command("git", function(argv, current)
  if #argv == 1 then return { "add", "commit", "push", "status" } end
  return oslo.completion.files(current)
end)

--------------------------------------------------------------------- suggestions
oslo.suggest = {
  sources = { "history", "completion", "path" },   -- fish's three, in order
  accept  = "right",                                -- key that takes it whole
  accept_word = "alt-right",
}

--------------------------------------------------------------------- keys
oslo.keys = {
  ["ctrl-r"]    = "history-search",
  ["shift-tab"] = "toggle-language",       -- what $OSLO_TOGGLE_KEY was
  ["ctrl-l"]    = "clear-screen",
}

--------------------------------------------------------------------- shell
oslo.opts.default_mode = "sh"              -- or "lua"
oslo.history = { size = 50000, file = "~/.local/share/oslo/history",
                 ignore_space = true, ignore_dups = false }

oslo.alias("ll", "ls -la")
oslo.set_var("EDITOR", "nvim")

oslo.on.precmd(function(cmd) --[[ … ]] end)
oslo.on.cd(function(dir) --[[ … ]] end)
```

Most of the right-hand column above does not exist yet. `oslo.alias`, `oslo.set_var`,
`oslo.on.*`, `oslo.opts`, `oslo.fs.cwd` and `oslo.exit_code` do; `oslo.theme`, `oslo.prompt`,
`oslo.completion`, `oslo.suggest`, `oslo.keys`, `oslo.history`, `oslo.style`, `oslo.git` and
`oslo.path.shorten` do not.

---

## Built

| Decision | Where |
|---|---|
| `~/.oslorc` is Lua | `startup/lua_init.rs`; `rc.rs` no longer sources it. Search order: `$XDG_CONFIG_HOME/oslo/config.lua`, `…/config`, `~/.oslorc` — first that exists |
| Lua theme table | `interactive/theme/` — roles, colour parsing (name/hex/256), degradation to 256/16/none, merge-not-replace reading with per-path diagnostics |
| Badges, no icons | `dropdown/render.rs` — kind as a coloured pill, column-aligned; the emoji icons are gone |
| Match highlighting | The typed prefix in the match colour, case-insensitively, on a character boundary |
| Syntax to fish's depth | `interactive/highlight/` — 16 token types, split into a lexer (no disk) and a classifier (needs the shell) |
| Autosuggestions | history → completion → path, order from `oslo.suggest.sources` |
| Prompt | `oslo.prompt.left/right/continuation`, plus `oslo.style`, `oslo.git.branch/root`, `oslo.path.shorten/home` |
| Right prompt | Drawn from the *highlighter*, verified on a real pty |
| Settings | `interactive/settings.rs` — `oslo.completion`, `oslo.suggest`, `oslo.history`, `oslo.keys`, pre-created as empty tables so `oslo.completion.max_rows = 5` is an assignment and not an index of nil |
| Info columns | `dropdown/columns.rs` — file → size, dir → entry count, alias → expansion. Computed at render time for the *visible* rows, so a `stat` per row is ~15 a frame and not 3,000 |
| Columns from Lua | `lua/columns.rs` — `oslo.completion.columns` returns a list per candidate; the layout fits however many there are, and the candidate carries every fact raw *and* rendered |
| Keys | `interactive/keys.rs` — named keys and named actions, both reported when unreadable |

### The right prompt, resolved

The plan called it *a hypothesis, not a plan*, and the first attempt was wrong. Putting the right
prompt in the **prompt string** corrupts rustyline's cursor arithmetic: it counts a CSI sequence as
zero width but has no idea that `\x1b[76G` *moves* the cursor, so the right prompt's own characters
get added to the column it thinks the line starts at. Measured, a six-cell prompt came out as
eleven.

The seam that works is the **highlighter**. `compute_layout` measures the raw line and never the
string `highlight` returns, so a cursor move there costs nothing; and `refresh_line` clears the old
rows *before* it writes and never after, so the drawing survives each redraw. Verified under a pty:

```
L> ^[[1;32mecho^[[0m ^[7^[[76GRIGHT^[8^[[90mhi^[[0m ^M^[[8C
```

The cursor lands at column 8 — correct for `L> ` plus `echo h` — so the right prompt did not
disturb it, and the ghost hint still draws at the cursor.

## Settled provisionally

The goal was to implement the plan, not to re-open it, so the questions below were answered the
least-surprising way and are **free to change**. None of them was a maintainer decision.

1. **`$ENV` stays POSIX shell.** POSIX defines it as shell syntax and oslo has to be a real
   `/bin/sh`; `~/.oslorc` going Lua does not change what `$ENV` is.
2. **`init.lua` is gone**, not kept as a fallback — the project rule is no legacy shims, and
   `~/.config/oslo/config.lua` is the same place under a name that matches the others.
3. **Config is interactive-only**, as bash's rc is. A theme has nothing to do in a script.
4. **`valid_path` is capped at 8 lookups per line** and off past 512 characters, rather than
   cached: the answer changes when the filesystem does, and nothing watches that.
5. **Colour degradation is automatic** from `$COLORTERM`/`$TERM`, with `$NO_COLOR` winning.
6. **A theme value may be** `"green"`, `"#61ffca"`, `"240"`, or a table with
   `fg`/`bg`/`bold`/`dim`/`italic`/`underline`/`reverse`.
7. **`oslo.theme` merges.** Naming one colour keeps the other forty.

## Open

Not decided. Each changes what gets built.

Still genuinely undone, rather than decided one way:

1. **`oslo.completion.for_command`** — a Lua completion for one command. The dropdown reads its
   candidates from Rust only; there is no hook for a config to add its own.
2. **`oslo.completion.sources` and `.sort`** are read but not acted on. `case_sensitive` likewise.
3. **`oslo.history` is read but not wired** to the editor: `size` and `file` still come from
   `$HISTSIZE` and `$HISTFILE`.
4. **`oslo.suggest.accept` / `.accept_word`** — the keys that take a suggestion are bindable
   through `oslo.keys`, but not under those names.
5. **Abbreviations** remain out of scope, as decided.
