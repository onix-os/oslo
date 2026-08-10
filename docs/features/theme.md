# Colours

One theme, read from `oslo.theme` once the config has run, used by everything the shell draws: the
line as you type it, the completion dropdown, the built-in prompt and the input widgets. It exists
because the first three were each carrying their own hardcoded escapes, which is how a shell ends
up with two different ideas of what a builtin looks like.

## How it works

A theme is 54 roles in four groups. Each role is a `Style` — an optional foreground, an optional
background, and five attributes (`bold`, `dim`, `italic`, `underline`, `reverse`).

| group | roles | what it paints |
|---|---:|---|
| `syntax` | 23 | the command line as it is typed |
| `pager` | 19 | the completion dropdown: 11 for the rows, 8 for the kind pills |
| `prompt` | 7 | the built-in prompt, when no Lua prompt has replaced it |
| `ui` | 5 | `oslo.ui` in Lua, the `ui` builtin in scripts |

**Merged, not replaced.** A config that writes `oslo.theme = { syntax = { command = "cyan" } }`
means "make commands cyan", not "discard the other fifty-three". Every field is an `Option` layered
over `Theme::default`, and only what the config names is overridden — a whole-table assignment is
the natural way to write one field and it must not blank the pager.

```
 startup ─────────────────────────────────────────────────────────────────────
   $COLORFGBG if set, else OSC 11 \x1b]11;?      100 ms bound, 20 ms settle
        └─ 0.299r + 0.587g + 0.114b < 0.5 ? Dark : Light
                 └─► theme::set_background()     changes what Syntax::default() IS
                                │
 the config runs ───────────────▼─────────────────────────────────────────────
   Theme::default()             the dark palette, or the light one
     + oslo.theme               only the fields the config named
     + inheritance              builtin, function ← command
                                repair ← autosuggestion, reversed
     + complaints ──► stderr    "…syntax.command: 'chartreuse' is not a colour"
     → theme::install()         once, after every config file has run
                                │
 every painted span ────────────▼─────────────────────────────────────────────
   theme::current()             a read lock and a clone of a plain struct
   theme::depth()               decided once from the environment, then cached
   Style::open(depth)           "\x1b[1;38;5;212m"
```

### Inheritance

Two rules, both there so that a two-line theme does not look half-finished.

`builtin` and `function` fall back to `command` when the config sets `command` and does not name
them. That is fish's rule: setting `command` alone recolours all three kinds of "this is the thing
being run". A config that names neither keeps both defaults, which is why the fall-back is
conditional on `command` having been written rather than on the values happening to match.

`repair` falls back to `autosuggestion` **reversed**. The correction drawn after a mistyped line is
the ghost's own colour turned inside out, so recolouring the ghost drags the correction with it
rather than leaving two unrelated greys on one line.

Separately, `quote` is still read and sets `single_quote` and `double_quote` together, so a config
written before those were split still means what it meant. It is detected with a sentinel no config
can write, because comparing against the current value cannot tell a config that set `quote` to the
default from one that never set it.

### Degradation

A 24-bit colour sent to a terminal that does not understand `\x1b[38;2;…` is not ignored — it
prints as literal digits across the prompt. So the depth is decided once and every colour comes
down to it.

```
  Rgb(0x50, 0xfa, 0x7b)   a theme's 24-bit green
    │
    ├─ True      38;2;80;250;123    as written
    ├─ Ansi256   38;5;n             the grey ramp when r == g == b, else the 6×6×6 cube
    ├─ Ansi16    37                 nearest of the sixteen, by squared distance —
    │                               which for this green is plain white
    └─ None      ""                 no escape at all, not an empty \x1b[m

  Depth::detect()   $NO_COLOR set                ──► None
                    $TERM empty or "dumb"        ──► None
                    $COLORTERM truecolor | 24bit ──► True
                    $TERM contains 256 | direct  ──► Ansi256
                    otherwise                    ──► Ansi16
```

Greys are checked before the cube because the cube's grey diagonal is coarse: quantising `#808080`
into it gives a visible tint, where the 24-step grey ramp has it almost exactly. The same `Depth`
paints `oslo --help` and the CLI's warnings, with one extra question there: whether stdout is a
terminal, which the environment cannot answer.

### A light background changes the default, not the theme

The dark palette is unreadable on white — `#50fa7b` on white is a pale green nobody can see.
`Syntax::for_light_background` is the same roles at darker values: sharpened, not lifted, because
"brighter" on a white background means *more colour*, not more light.

**It is installed as the default rather than as a theme, and the ordering is the whole point.** The
background is decided before the config runs, so `Syntax::default()` already answers with the light
palette by the time `read_lua_theme` starts merging: a config that names three colours gets the
light palette for the rest, and its own three still win. Installing a theme at detection time
instead is overwritten wholesale the moment the config is read, which is what happened on the first
attempt. A terminal that stays silent keeps the dark palette, which is the safer guess.

### Brightening

The syntax palette is absolute RGB rather than the sixteen ANSI slots, because a palette tool like
pywal remaps what slot 2 means and a theme built on the slots would change colour whenever the
wallpaper did. Being absolute is also what makes it adjustable: every hue is passed through
`Color::intensified` before use — `+0.12` on HSV's value axis for a dark background, `0.0` for a
light one, `+0.18` on saturation either way. HSV rather than HSL, because raising HSL lightness
past the midpoint bleeds chroma toward white: `#ff5555` becomes `#ff8888`, a *paler* red.

**`intensified` returns an ANSI slot unchanged.** `Basic` means "colour 2, whatever this terminal
thinks that is", and rewriting it to an absolute value would be oslo overruling a choice made
somewhere else entirely. Near-greys, below 0.15 saturation, are left alone too, which keeps the
dropdown's chrome, the black on `sudo`'s red field and the ghost-text grey from being "brightened"
into worse versions of themselves.

### Named styles

Separate from the roles, and for a different job: `oslo.theme.styles` maps a name to a style, and a
prompt segment then writes `style = "git.branch"` and never mentions a colour. The spelling is
hexe's — space-separated words, `fg:` and `bg:` for colours, bare words for attributes — and a bare
colour with no prefix is a foreground. An unreadable word is skipped rather than refusing the whole
string, because a style is written by hand and a typo in one attribute should not blank the prompt
piece.

A dotted name the config has not defined resolves against the theme's `prompt` group: `prompt.cwd`,
`prompt.host`, `prompt.user`, `prompt.git`, `prompt.ok`, `prompt.failed`, `prompt.aside`. Anything
else is parsed as a colour, so `"cyan"` works without a theme having been defined first. A config's
own definition outranks both.

### The test that catches a role with no reader

A role arrives in two halves: the field in the Rust struct, and the line in `from_lua` that reads
it. Forgetting the second produces no compile error and no failing test — only a config that
appears to be ignored. `prompt.host` and `prompt.user` lived like that, defaulted and drawn but
reachable only through the `oslo.theme.styles["prompt.host"]` back door, while the obvious spelling
silently did nothing.

`every_role_can_be_set_from_a_config` sets all 54 in one Lua table, with a distinct value per role,
and asserts that none of them still equals its default; anything with no reader is named in the
failure. The group counts are asserted too — `23`, `11 + 8`, `7`, `5` — so adding a role forces the
list to be extended rather than quietly slipping past.

## What makes it different

The role names are fish's, deliberately, because they are the ones people already have written down
somewhere and because the set is a good specification of how deep highlighting should go.

**Only the syntax palette is pinned to absolute RGB.** The prompt, the pager rows and the widgets
keep the basic ANSI slots on purpose, so they still follow whatever scheme the terminal is using —
which is the difference between a prompt that sits in your theme and one that ignores it.

The widget palette is one accent plus ordinary ANSI, and the smallness is the design. gum has a
flag for the colour of every part of every widget; the result is that nobody sets any of them, and
the few who do end up with a prompt that matches nothing else on the screen.

## Configuration

```lua
oslo.theme = {
  syntax = { command = "#7cff9d", keyword = "212",
             comment = { fg = "244", italic = true },
             ['function'] = "cyan", ['end'] = "244" },   -- Lua keywords: bracket form
  pager  = { bg = "#101010", sel_bg = "238" },
  prompt = { cwd = "blue", git = "green" },
  ui     = { accent = "213" },
}
```

Four ways to write a colour, all accepted wherever one is: a name (`"green"`), a bright name
(`"brightblack"`, or fish's `"brblack"`), a hex triplet (`"#61ffca"`, or the CSS short form
`"#0f8"`), or a 256-colour index (`"240"`). `"normal"` and `"default"` mean the terminal's own.
Anything else is refused and reported by path, rather than silently becoming black — a
black-on-black prompt is far harder to diagnose than an element that did not take its colour.

An entry is a string or a table: `"green"` and `{ fg = "green", bold = true }` are the same kind of
value, the short form being what people write and the long form what they need the moment they want
it bold. `pager.bg`, `pager.sel_bg` and `pager.kind_sel` are bare colours rather than styles — they
are a row's background and nothing else.

The roles, by group:

| group | names |
|---|---|
| `syntax` | `command` `builtin` `function` `keyword` `error` `danger` `param` `valid_path` `option` `glob` `number` `assignment` `single_quote` `double_quote` `escape` `operator` `redirection` `end` `comment` `variable` `autosuggestion` `repair` `match_bracket` |
| `pager` | `bg` `text` `text_sel` `sel_bg` `kind_sel` `match` `desc` `desc_sel` `extra` `extra_sel` `scroll`, and `kind.{command,builtin,file,dir,variable,history,alias,other}` |
| `prompt` | `cwd` `host` `user` `git` `ok` `failed` `aside` |
| `ui` | `accent` `question` `muted` `error` `done` |

Named styles, and the depth when detection is wrong:

```lua
oslo.theme.styles["my.dir"] = "fg:#8be9fd bold"
oslo.theme.styles["prompt.error"] = "bg:1 fg:0"

oslo.misc.color_depth = "truecolor"   -- truecolor / 256 / 16 / none
```

`color_depth` is applied when the settings are installed rather than where colour is painted,
because the depth is cached on first use and a config that set it after something had already drawn
would be ignored.

## Measurements

`cargo bench --bench keystroke`, on this machine:

| | µs |
|---|---:|
| paint a 57-character line, per keystroke | 2.17 |

That is the whole syntax pass — classify, then one `Style::open` per token — and it is what a
repaint spends its time on. `theme::current()` is a read lock and a struct clone; `theme::depth()`
is cached, because it is read once per styled span and a full dropdown redraw asks several hundred
times.

## What it cannot do

- **Survive `oslo.theme = { … }` if you also use `oslo.theme.styles`.** The whole-table assignment
  replaces the table that carries the `styles` registry and its `__newindex`, so a later
  `oslo.theme.styles["x"] = "fg:red"` dies with *attempt to index a nil value* and takes the rest
  of the config file with it. Set the fields instead — `oslo.theme.syntax = { … }` — or write the
  styles first.
- **Adapt anything but the syntax palette to a light background.** `pager`, `prompt` and `ui` have
  one set of defaults; the dropdown's `Indexed(236)` field stays dark on a white terminal unless a
  config says otherwise.
- **Notice the terminal changing.** The background is asked once, before the first prompt, and only
  by an interactive shell whose `$TERM` is not `dumb`. Switching your terminal from dark to light
  mid-session changes nothing until the next shell.
- **Be reloaded.** The theme is read out of the interpreter once, after every config file has run,
  and installed. There is no path that re-reads it.
- **Be named.** `oslo.theme = "dracula"` is reported as *must be a table*. There is no theme file,
  no registry and no import.
- **Keep `$NO_COLOR` if the config overrides the depth.** `$NO_COLOR` beats detection, but
  `oslo.misc.color_depth` is applied afterwards and wins over both.
- **Express more than five attributes.** No blink, no strikethrough, no underline colour, no
  per-attribute colour.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-ui/src/theme/mod.rs` | `Theme`, `Syntax`, `Pager`, `Prompt`, `Ui`, the defaults, `current`/`install`/`depth`/`set_background` |
| `crates/oslo-ui/src/theme/from_lua.rs` | `read_lua_theme`, the inheritance rules, `every_role_can_be_set_from_a_config` |
| `crates/oslo-ui/src/theme/color.rs` | `Color`, `Depth`, `Style`, `Depth::detect`, quantisation, `Style::open` |
| `crates/oslo-ui/src/theme/color/vivid.rs` | `Color::intensified`, the HSV gains |
| `crates/oslo-ui/src/theme/styles.rs` | the named-style registry and hexe's spelling |
| `crates/oslo-ui/src/term/query.rs` | OSC 11, `parse_background`, `$COLORFGBG`, the reply broker |
| `crates/oslo-ui/src/term/negotiate.rs` | the one startup exchange the OSC 11 query rides in |
| `crates/oslo-runtime/src/startup/terminal.rs` | asks, then `theme::set_background` |
| `crates/oslo-runtime/src/startup/config.rs` | reads the theme out of the interpreter, installs it |
| `crates/oslo-runtime/src/lua/api/prompt.rs` | `oslo.theme.styles`, `style_named` |
| `crates/oslo-ui/src/highlight/mod.rs` | `paint`, where the syntax roles are applied |
| `bench/keystroke.rs` | the number above |
