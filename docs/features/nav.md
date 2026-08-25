# The filesystem navigator

`nav` is a builtin that draws the directory you are standing in, lets you walk around it by typing,
and leaves the shell in whatever directory is on screen when you press Escape. It is a builtin
because a separate process cannot reach into its parent and change the working directory: the most
an external browser can do is *say* where it ended up, and something inside the shell has to read
that and act on it. `nav` is that something — see [browsing with something else](#browsing-with-something-else).

```sh
nav              # start here
nav /var/log     # start somewhere else
```

<!-- demo:begin -->
[![nav demo](https://asciinema.org/a/1262742.svg)](https://asciinema.org/a/1262742)
<!-- demo:end -->

## Browsing with something else

`nav` draws its own browser unless the config names another one:

```lua
oslo.builtin.nav.command = { "trek", "--explore", "--cwd-file", "{answer}", "{dir}" }
```

Everything around it is unchanged — same name, same operand, same effect on the shell:

```sh
nav              # whatever `command` names, or oslo's own when it names nothing
```

The split is the point. `nav`'s job is to leave the shell in the directory you chose; drawing the
thing you choose in is a *different* job, and one a dedicated tool may do better — trek has tabs,
git status and a commit graph. So the builtin keeps the half only a builtin can do and hands over
the half anyone can.

**A setting rather than a search of `$PATH`.** This used to look for `trek` and take over whenever
it happened to be installed. A program changing what a shell builtin does by existing is a surprise
nobody asked for, and one that could not be turned off without uninstalling something. oslo's
browser is the default and stays the default; choosing another is an act.

**A browser that draws elsewhere keeps the prompt alive.**

```lua
oslo.builtin.nav.detached = true
```

Run inline, a browser takes the terminal: oslo's prompt is not visible and there is nothing to keep
alive. Opened in a terminal mux's float it is the opposite — oslo's own screen sits there with a
prompt on it for the whole visit, and without this that prompt is frozen. An animated segment stops
moving, and a directory the browser moves the shell to over
[the control socket](control-socket.md) is not drawn until you come back.

With it on, `nav` polls the browser rather than blocking on it, and between polls does what the
editor's loop does with the keyboard taken out: service the background, and redraw the prompt if
that changed anything. A browser can then walk the shell around and the prompt beside it says so as
it happens.

**Off by default, because being wrong about it damages the screen** — a prompt repainted over an
inline file list lands in the middle of it. oslo cannot tell the two apart; both are a child that
blocks. So the config says which.

Two things it does not do. `$PS1` is not rebuilt while a browser is up: building it needs the shell
state, and the browser is the thing holding it — a prompt written in Lua or handed to another
program is rebuilt, which is every prompt this was built for. And a `cd` arriving mid-visit moves
the *kernel's* idea of where the shell is, so the prompt shows it, while `$PWD`, the directory ring
and `post-change-dir` are finished at the next safe point rather than half-done early.

**A browser that is not installed here is no browser.** One config is read on every machine you log
in to, and the one it names is not on all of them — so a `command` whose program cannot be found
falls back to oslo's own browser rather than failing. Nothing is printed: the fallback is the
behaviour, not a degraded one. Anything else that stops the spawn — a name that exists but is not
executable — is a broken configuration and does say so, because silently drawing something other
than what was asked for would leave nothing to debug with.

### The placeholders

| | |
|---|---|
| `{answer}` | a private file the browser **must** write its final directory into |
| `{dir}` | where to start |
| `{width}`, `{height}` | `oslo.builtin.nav.width` and `.height`, defaulting to 60×50 |

They substitute *inside* an argument, not only as the whole of one — which is what lets a nested
command line carry them through to the program that finally runs. That is how a browser is opened
in a terminal mux's float:

```lua
-- in a hexe session: a float, so the shell behind it stays on screen
oslo.builtin.nav.command = {
  "hexe", "mux", "float",
  "--command", "trek --explore --cwd-file {answer} {dir}",
  "--cwd", "{dir}", "--title", "trek", "--size", "21,81", "--pass-env",
}
```

`hexe mux float` waits for the float to close, so `nav` reads the answer and `cd`s exactly as it
does for a browser that ran inline. Drop `{width}`/`{height}` there: a float already has a size, and
two things deciding one layout is one too many.

### The file they talk through

The browser writes the directory it finished in; `nav` reads that back and `cd`s there. The file is
created inside a private `0700` directory made for that one run, **not** at a predictable path under
`/tmp`: the shell goes wherever that file says, so a path a stranger could create first is a path
that chooses your working directory.

Writing nothing there is a cancelled navigation, which is what oslo's own browser reports when you
leave it with Escape. The browser's exit status is `nav`'s.

Its own settings are its own. `nav`'s — the border, the filter row, hidden files — describe the
builtin browser and do not apply; a tab, a keybinding or an icon belongs in that tool's config, not
here.

## How it works

One loop: read the directory, draw a frame, wait for a key. Nothing is carried across directories —
arriving anywhere, or re-reading where you are, throws the width measurement away with the listing
— and nothing runs in the background.

The frame is the path, the listing, and — only once you are filtering — a filter row underneath:

```
      ┌ pad(1) + marker(2). The heading is inset by exactly this, so the path
      ▼ starts where the names start rather than three cells to their left.
      /home/bresilla/data/code/tools/rush   [all]    ← where you are; [all] = hidden on
    ■  crates/                              4.0K   2d
  > ■  docs/                                4.0K  17h  ← the row the cursor is on
    ≡  Cargo.toml                           892B    3d
    ≡  README.md                             31K    1h
    │  │                                    │       │
    │  └ name; directories first, then by name; `/` for a directory, `@` for a link
    └ the mark, one cell, chosen by extension                 size ┘   age ┘  hard right

  >>  filter @ doc                                     1/47   ← only while filtering
```

The `dir`/`file` word and the `rwxrwxr-x` mode that a first version drew are both gone. **A file
browser is read down the names**, and everything to the left of them is something the eye has to
skip on every row to get there; nine characters of mode is a question a browser is rarely asked,
and the kind is already said by the trailing `/`. Size and age moved hard right, where they form
two columns of their own. The mark that replaced them is one cell and is configuration.

Sizing follows the same rule vertically and horizontally: **half the terminal is a ceiling, not a
size.** A directory of nine entries asking for twenty-four rows drew fifteen blank ones, and a
hundred-column box holding forty-six columns of listing put fifty blank cells between every name
and its age. The width is measured across *every* entry rather than the ones the filter currently
matches, and cached per directory — otherwise the block would resize and re-centre under the cursor
on each keystroke, and a directory of ten thousand files would be formatted once per keypress. An
empty directory has no rows to measure, so the path counts towards the width too; without that, the
whole widget collapsed to a single `…` the moment you opened somewhere empty.

### What a key does

There are three modes and one key routing table:

```
key
 │
 ├─ a character, less than settle_ms after an automatic walk ────────► dropped
 │
 ├─ Delete mode   y Y Del → remove    n N Esc → back    Ctrl-C → quit
 │
 ├─ Filter mode   Esc → cd + quit     Ctrl-C → quit, shell unmoved
 │                Enter, → → open     ← → parent
 │                ↑ ↓ PgUp PgDn Home End → move
 │                Del → ask before removing the selected entry
 │                Backspace → shorten (empty ⇒ Browse)     Ctrl-U → Browse
 │                ? → legend          any other character → filter, maybe walk in
 │
 └─ Browse mode   all of the above, except
                  . → toggle hidden files and re-read
                  any other character → start a filter with it, maybe walk in
```

`.` toggles hidden files **only while browsing**. A dot is the commonest character in a filename —
`Cargo.toml` cannot be typed if it always means something else — so once a filter is being typed it
goes into the filter like anything else. Before that there is nothing it could be part of, which is
exactly where the shortcut belongs. `?` has no such escape: it toggles the legend in both modes.

Escape and Ctrl-C are the two ways out and they differ in one thing only: Escape returns the
directory on screen and the builtin `cd`s to it, Ctrl-C returns nothing and the shell stays where
it was. The exit status says which: the `cd`'s own status after a change, 1 for a cancel, 2 when
there is no terminal at all.

### Typing a name walks into it

The filter is meant to read as a path being typed rather than a search being run, so when it leaves
exactly one match and that match is a directory, `nav` goes in without waiting for Enter. That
creates a problem the timer solves:

```
  f     u     z          z           ← you are typing "fuzz"
  │     │     │          │
  │     │     └ only `fuzz/` left → load it, and start the settle clock
  │     │                  ┌──── settle window, 500 ms by default ────┐
  └─────┴ narrowing        └ the trailing z arrives HERE, and is dropped
```

The word is usually longer than the prefix that identifies it. `fuzz` stops being ambiguous at
`fuz`, so the walk happens with a `z` still to come, and that `z` would otherwise start a search in
a directory nobody has looked at yet — which is exactly what the test
`without_a_deadline_the_trailing_character_leaks_into_the_new_directory` pins, by setting the
deadline to zero and asserting the leak.

Three rules keep it from being annoying:

| rule | why |
|---|---|
| directories only | opening a file means choosing a program, and being wrong about that costs more than a keystroke |
| never on an empty query | a directory holding one entry would otherwise swallow you the moment you arrived |
| only characters are dropped | Escape, the arrows, Enter and Backspace all work through the window, so it can never read as a widget that has stopped responding |

Backspacing down to a single match does not walk in either — only a typed character can, so
deleting never pushes you forwards.

### Delete goes through `rm`

The confirmation is drawn by the navigator; the removal is not. `nav` calls oslo's own `rm` builtin
with the path, which means the deletion policy of the shell is the deletion policy of the browser:
with `oslo.builtin.rm.to_tmp` on, the entry is moved to the trash directory and the size cap
applies, and with it off the entry is unlinked. **There is one implementation of "delete" in oslo
and this is not a second one.** If `rm` reports failure the listing keeps the entry and the widget
says `delete failed`; on success the directory is re-read in place.

## What makes it different

A file browser run as its own process cannot move the shell that started it — a child cannot change
its parent's working directory — so it has to hand the chosen path back out of band and have the
shell `cd` to it afterwards. `nav` is a builtin holding a mutable `Environment`, so the change of
directory is a call to the same `cd` any other line would use. No temporary file, no wrapper
function, nothing to install.

The other consequence of living inside the shell is deletion. An external browser brings its own
notion of what removing a file means, and now the machine has two. `nav`'s Delete is `rm`, so it
inherits `oslo.builtin.rm` exactly.

The nearest thing inside oslo itself is completion on `cd`, which offers the names of a path you
are already typing. It completes a *line*, and you are still where you were until you press Enter;
`nav` moves as you type and shows the size and age of each candidate while you choose.

## Configuration

Everything is under `oslo.builtin.nav`. `command` is the one that decides *which* browser — see
[browsing with something else](#browsing-with-something-else); the rest describe oslo's own. The two
knobs that are not in the README's settings block:

```lua
oslo.builtin.nav.icons = {
  dir = "■", file = "≡",
  ext = { rs = "r", md = "m", toml = "t" },
}

oslo.builtin.nav.type_nav = { enabled = true, settle_ms = 500 }
```

**Assign the whole table.** `oslo.builtin.nav` is pre-created so a config can index it, but
`oslo.builtin.nav.icons` is not: `oslo.builtin.nav.icons.ext = {...}` indexes a nil value and takes
the rest of the config file down with it.

**Two marks are built in and no more, and `ext` starts empty.** Which glyph a `.rs` deserves is a
matter of taste, of the font you run, and of what you work on, so it belongs in a config file
rather than in the source. The two defaults are geometry rather than glyphs from a patched font,
because they have to land in a terminal that has never heard of Nerd Fonts. Extensions are matched
without regard to case, and a name that merely begins with a dot has no extension — `.gitignore` is
not a `gitignore` file. A directory always takes `dir`, even if it is called `src.rs`.

The presentation settings, with their real defaults:

| setting | default | |
|---|---|---|
| `fullscreen` | `true` | the alternate screen; `false` draws inline |
| `position` | `"center"` | `top` / `center` / `bottom`; vertical, and only on a full screen |
| `width`, `height` | `0` | zero means half the terminal, capped by the listing; an inline `height` of zero is capped at fourteen rows instead |
| `border` | `"none"` | `none` / `rounded` / `square` / `double` / `thick` |
| `border_fg`, `border_fit` | `nil`, `"content"` | |
| `legend` | `false` | whether the key legend starts shown; `?` toggles it either way |
| `legend_gap`, `padding_x`, `padding_y` | `1`, `1`, `0` | |
| `hidden` | `false` | whether dotfiles are listed on opening; `.` toggles it |
| `filter_at` | `"bottom"` | which end the filter row sits at |
| `reverse` | `false` | **downward, because the path is above it** — a reversed list leaves its unused rows at the top, which here lands them between the path and the first entry |
| `scanner` | `true` | the animated sweep on the filter row |

The filter's matching is not a `nav` setting: it reads `oslo.completion.fuzzy`, which is
`off` / `tight` / `smart` / `loose`, so the browser narrows names the same way the completer ranks
them.

## What it cannot do

This describes the builtin browser. With `oslo.builtin.nav.command` set, the list is that program's.

- **Be scripted.** There is no `oslo.nav()` and no way to get the chosen path into a variable;
  `nav` changes the shell's directory and that is its only output. It takes one optional path and
  no options but `-h`/`--help`.
- **Type a path.** `/` is an ordinary character and matches nothing, so a multi-segment path is
  typed one segment at a time — and typing the next segment inside the settle window loses the
  characters, because dropping them is what the window is for.
- **Open anything.** Enter on a file does nothing. There is no copy, move, rename, mkdir or
  preview, and no marking: one entry is selected and that is all.
- **Say what it deleted.** Delete passes no `-r`, but at an interactive prompt oslo's `rm` removes
  a directory without it, so confirming on a directory removes the whole tree — and the question
  only names the entry. The widget's own report is the two words `delete failed`; the reason comes
  from `rm`.
- **Notice changes.** The listing is read when you arrive, when you toggle hidden files, and after
  a delete. Nothing watches the directory, and a large one is read and sorted in full before the
  first frame.
- **Filter on `?`.** The key is the legend toggle in every mode.
- **Preserve a symlinked path.** The directory is canonicalised, so `nav` leaves you at the
  physical path where `cd` through a symlink would have kept the logical one.
- **Run without a terminal.** No terminal means status 2 and `oslo: nav: no terminal available`.
- **Show a mark wider than one cell.** The marks are right-aligned as a block with the row measured
  against them, and an emoji is two cells — one emoji among the marks starts that row's name a
  column over from every other.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-ui/src/nav.rs` | `Navigator`, `Outcome`, `State`, the key loop, the sizing |
| `crates/oslo-ui/src/nav/listing.rs` | `Entry`, `read`, `row_of`, `narrow` |
| `crates/oslo-ui/src/nav/tests.rs` | the type-and-navigate tests, including the leak |
| `crates/oslo-ui/src/settings/nav.rs` | `Nav`, `TypeNav`, `Icons`, `Icons::of` |
| `crates/oslo-ui/src/settings/from_lua.rs` | reading `oslo.builtin.nav` |
| `crates/oslo-shell/src/env/builtins/nav.rs` | `builtin_nav`, the `rm` closure, `change_directory` |
| `crates/oslo-shell/src/env/builtins/remove.rs` | what Delete actually calls |
| `crates/oslo-ui/src/ask/look.rs` | `Preset::History`, `Look`, `Step` — the rows and the arrows |
| `crates/oslo-ui/src/ask/chrome.rs` | the border, the placement, the legend |
