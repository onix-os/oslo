# Abbreviations

A short word that becomes a long one as you type the space after it: `gco ` leaves
`git checkout ` in the buffer. It exists because the obvious tool for the job — an alias — is a
language feature, and a shell that promises scripts see POSIX behaviour should not reach for a
language feature to save four keystrokes.

<!-- demo:begin -->
[![abbreviations demo](https://asciinema.org/a/1262733.svg)](https://asciinema.org/a/1262733)
<!-- demo:end -->

## How it works

The space key is the trigger, and it is handled *before* the space is inserted, because the
expansion supplies its own: `gco ` becoming `git checkout ` has to be one edit, or you would watch
the line flicker through a word waiting to be finished.

```
you type:   g   c   o   ␣
                        │
                        └─ Action::Insert(' ')      the space is NOT in the buffer yet
                             │
                             ▼
                     ShellAssist::abbreviation(line, cursor)
                             │
                             ├─ feature "abbr" masked off ─────────► None ──► plain space
                             ├─ char cursor → byte offset (the editor counts characters,
                             │                              abbr::expand counts bytes)
                             ▼
                     abbr::expand(text, cursor)
                             │
                             ├─ word = back to the last whitespace          "gco"
                             ├─ one HashMap lookup                miss ────► None
                             ├─ placement is Command, and this is not
                             │  a command position                    ─────► None
                             └─ hit:  text[..start] + expansion + text[cursor..]
                                      cursor = start + (byte index of '%',
                                                        or the end of the expansion)
                             │
                             ▼
                     the triggering space is inserted at that cursor
                             │
                             ▼
                     buffer.set("git checkout ", 13)
```

Answering `None` is the answer for almost every space ever typed, so the cost of the feature on the
keystroke path is one hash lookup on a table that is usually empty.

### Where it may fire

The default placement is `command` — only where a command name would go — because `gco` meaning
`git checkout` in the middle of a filename is a surprise nobody asked for. The test for a command
position is deliberately not a parse:

```
is_command_position(text before the word)
  │
  ├─ nothing before it                                    → yes   `gco`
  ├─ the previous word is one of
  │    sudo  doas  command  time  env  xargs              → yes   `sudo gco`
  ├─ the text ends with  |  &  ;  (  {  &&  ||          → yes   `ls | gco`, `true && gco`
  └─ anything else                                        → no    `cat gco`
```

**Being wrong here costs a keystroke, not a mistake.** If the test says no when a real parser would
have said yes, the abbreviation does not fire and you type the long form — which is what you would
have done without the feature. That asymmetry is the whole argument for keeping the test to six
words and a suffix check rather than running the parser on every space.

### `%` places the cursor

`abbr gc 'git commit -m "%"'` leaves the cursor inside the quotes. The marker is removed from the
expansion, and only the first one is: there is no escape for a literal `%`, and a second `%` is
ordinary text.

### What actually lands in the line

This is the point of the feature, and it is worth being precise about. The expansion is inserted
into the buffer as text, before anything is parsed. Everything downstream therefore sees the real
command:

| what sees it | `alias gco='git checkout'` | `abbr gco 'git checkout'` |
|---|---|---|
| the line on screen | `gco main` | `git checkout main` |
| the command log and `$HISTFILE` | `gco main` | `git checkout main` |
| what the finder recalls | `gco main` | `git checkout main` |
| what the prediction model learns (`oslo` only) | `gco main` | `git checkout main` |
| what runs | `git checkout main` | `git checkout main` |

The log is written from the line you accepted, so an alias puts the shortcut in your history and an
abbreviation puts the command there. That is not only tidiness: the model behind ghost suggestions
and repair learns whole lines, so a shell driven by aliases teaches it a private vocabulary that
means nothing to any other tool, and one driven by abbreviations teaches it the commands themselves.

The other half is that **you watch it happen**, which is how you end up knowing `git checkout`
rather than knowing `gco`.

### Quoting

`abbr::expand` splits on whitespace and knows nothing about quotes, which has two consequences that
are easier to state than to guess:

- A quote character attached to the word is part of the word, so `echo "gco ` looks up `"gco`,
  finds nothing, and does nothing. An abbreviation cannot fire immediately inside an opening quote.
- With `--anywhere` it *can* fire further inside a quoted string — `echo "hello gco ` expands —
  because there is no parser to say otherwise. `command` placement is nearly always safe there by
  accident: the preceding word is rarely one that starts a command.

The expansion itself is inserted literally, quotes and all, and is then parsed exactly as if you
had typed it. The `abbr` builtin sees its arguments *after* the shell has split and unquoted them,
so an expansion that must contain quotes needs to arrive as one quoted argument:

```sh
abbr gc 'git commit -m "%"'   # the double quotes survive into the buffer
abbr gc git commit -m "%"     # the words are joined with single spaces: git commit -m %
```

Joining the trailing words is deliberate — `abbr gst git status` is what someone reaches for first,
and refusing it teaches nothing.

## What makes it different

fish's `abbr` is the direct ancestor, and oslo takes two ideas from it: the placement distinction
and a marker for where the cursor lands. It takes no more than that — there are no regex-matched
names and no function-valued expansions here, only a word and the text it becomes.

Against oslo's own `alias`: an alias changes what a command *means*, it is still an alias after you
press Enter, and it is a name the shell resolves. An abbreviation is a keyboard convenience and
nothing else. `alias` is still there and still POSIX; use it when you want a command's meaning
changed, and an abbreviation when you only want to type less.

## Configuration

At the prompt, where it is the shorter thing to type:

```sh
abbr gco 'git checkout'                          # define
abbr gc 'git commit -m "%"'                      # % is where the cursor lands
abbr --anywhere brc '~/.config/oslo/config.lua'  # fires in any word, not just a command
abbr                                             # list, sorted, in a form you can paste
abbr -e gco                                      # remove (--erase also works)
```

In the config, for the ones you want every session:

```lua
oslo.abbr.gco = "git checkout"
oslo.abbr.gc  = 'git commit -m "%"'
oslo.abbr.brc = { "~/.config/oslo/config.lua", anywhere = true }
oslo.abbr.tmp = { expansion = "/tmp" }
```

The long form takes the expansion as its first element or as `expansion = …`; `anywhere = true` is
the only other key. An entry with no usable expansion is reported by name at startup rather than
dropped, because an abbreviation that does not fire looks exactly like the shell ignoring the
config.

The config seeds the same table the `abbr` builtin writes, rather than being consulted on each
keystroke — two sources that can disagree is worse than one that a config seeds. It clears the
table first, so a re-read does not leave behind an entry the config no longer defines, but only
when the config defines at least one, so a shell whose abbreviations all came from `abbr` at the
prompt does not lose them.

Both the expansion and the builtin are one runtime feature:

```lua
oslo.feature.set("abbr", false)   -- no expansion, and `abbr` is no longer a builtin name
```

## What it cannot do

- **Expand on Enter.** The space is the only trigger. `gco` followed by Enter runs `gco`, which is
  not a command. The rule is that what runs is what you had time to read.
- **Expand a paste.** A bracketed paste is a different input event and never reaches the space
  handler, so pasted text arrives exactly as it was.
- **Be seen by anything but the editor.** `type`, `command -v` and completion have never heard of
  an abbreviation, and no child process can. It is not a name; it is a typing shortcut.
- **Undo itself.** The expansion replaces the buffer outright and nothing remembers the short form,
  so backspacing gives you a shorter `git checkout `, not `gco `.
- **Have a name with a space in it.** The lookup is on a whitespace-delimited word, so such a
  definition is accepted and can never fire.
- **Survive the session** on its own — the table is in memory only. `oslo.abbr` is how one lasts.
- **Place the cursor and the trigger space independently.** The space that fired the expansion is
  inserted at the cursor, which is where `%` put it: `abbr gc 'git commit -m "%"'` typed as `gc `
  leaves `git commit -m " "` with the cursor after that space, inside the quotes.
- **Know which language the prompt is reading.** Unlike ghost suggestions, which answer for the
  current language, an abbreviation fires at a Lua prompt as readily as a shell one.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-ui/src/abbr.rs` | the table, `Placement`, `expand`, `is_command_position` |
| `crates/oslo-shell/src/env/builtins/abbr.rs` | `builtin_abbr` — define, list, `-e` |
| `crates/oslo-ui/src/edit/session.rs` | `Action::Insert(' ')`, the one trigger |
| `crates/oslo-ui/src/edit/session/assist.rs` | the `abbreviation` seam on `Assist` |
| `crates/oslo-runtime/src/startup/native.rs` | `ShellAssist::abbreviation`: the gate, the cursor conversion, the trigger space |
| `crates/oslo-ui/src/settings/from_lua.rs` | reading `oslo.abbr`, both forms |
| `crates/oslo-ui/src/settings/mod.rs` | `Settings::abbr` |
| `crates/oslo-runtime/src/startup/config.rs` | installing what the config declared |
| `crates/oslo-base/src/feature.rs` | `at::ABBR`, and `abbr` as a builtin the feature provides |
