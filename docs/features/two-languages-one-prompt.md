# Two languages, one prompt

oslo's prompt reads one language at a time — POSIX shell or Lua — and Shift-Tab, Ctrl-Space or Tab
twice switches between them without disturbing the line you are typing. It exists because a shell that
guessed the language by looking at the line would decide what `print(1)` means from whatever
happens to be installed, and a shell whose meaning depends on that is one you cannot write
scripts against.

<!-- demo:begin -->
[![two-languages-one-prompt demo](https://asciinema.org/a/1262753.svg)](https://asciinema.org/a/1262753)
<!-- demo:end -->

## How it works

The language is a single value in the read loop, `current` in
`startup::read::read_command`. It lives for the whole session: switching is a property of the
session, not of one line. A second value, `reading`, is the language *this* command is being read
in — it follows `current` until a `!` prefix at a shell prompt sends one line to Lua.

Everything outside the loop learns the language from a different place. `oslo-ui` keeps one
process-wide record of what the prompt on screen says, written by `prompt::note_row` before each
line is read and read back by `prompt::language`. That indirection is not decoration: **the
suggestion, the completion filter and the history finder all live in the library and have no way to
reach the loop's variable**, and the language can change in the middle of a line from a key handler
that cannot reach the editor either.

```
Shift-Tab  ─►  Key::BackTab  ─►  ShellAssist::binding  ─►  Bound::ToggleLanguage
                                        │
                        (an oslo.keys entry is consulted first, so
                         "none" cancels the default outright)
                                        ▼
                              Session::perform  ─►  Step::ToggleLanguage
                                        │
   read_line: repaint the CURRENT frame, move the cursor to the top of the
              block, return — the row never goes blank
                                        ▼
                    Outcome::ToggleLanguage { text, cursor }
                                        │
   read_command:  hide the cursor            \x1b[?25l
                  fire pre-mode-change       { kind = "language", from, to }
                  *current = current.other()
                  fire post-mode-change
                  reading = *current
                  typed = text                 ← the line, kept
                  typed_point = byte index of cursor
                                        │
                  ┌─────────────────────┘
                  ▼
   note_row(reading.name(), …)   ← every library-side lookup now answers for the new one
   render the prompt again        (same prompt; the language is one segment of it)
   read_line(render, (&typed, cursor), assist)
```

Three details in that path were each forced by a defect.

The editor **repaints the frame it already has** before returning, instead of erasing the block. It
used to erase, and the screen then held an empty row for however long the caller took to render the
other language's prompt — which for a prompt built by running another program reads as the whole
prompt flashing dark. The cursor is hidden across the gap for the same reason: `read_line` restores
the terminal on the way out, which makes the cursor visible again at column one while the next
prompt is being built.

The cursor position crosses the boundary twice converted: the editor counts **characters**, the
loop stores a **byte** offset in `typed_point`, and `read_line` is handed a character index again.
The two agree for ASCII and diverge on the first accented letter.

Both languages share one prompt with the language as a segment, and the built-in prompt is padded
to one width across every name in `prompt::LANGUAGES` (`["sh", "lua"]`). The editor is told the
prompt's width once, when the line starts, and every piece of arithmetic afterwards — where the
cursor is, where the ghost hint goes, where a wrapped row breaks — comes off that number. A `lua`
segment one cell wider than `sh` would put the text a cell away from where the editor believes it
is. The width is measured across the list rather than hard-coded, so adding a language cannot
quietly bring the shifting back.

### One prefix, and it goes one way

`!` is read off the first line of a **shell** command, after it has been trimmed and before
anything parses it, and only when something follows.

```
first physical line, typed at a prompt reading mode M
        │
        ├─ M = lua → nothing is read off it. Every line is Lua.
        │
        ├─ M = shell, line starts with '!', rest is not blank,
        │             and the rest does not open a history reference?
        │      yes ──► Line::OneOff { mode: Lua, text }   ← mode unchanged
        │      no  ──► Line::Normal(line)                 ← run as shell
        │
        └─ a continuation line is never re-examined: by then the language is decided
```

**Why `!` and not `=`.** `=` is a character a shell already spends: `FOO=bar` is an assignment in
every shell there is, `=cmd` is a real expansion in zsh, and oslo's own `=grep` answers where a
program lives — so a leading `=` was three things at once and the prompt had to pick one. `!` is
the shell's own reach-back character and only has to share with history expansion, which is a much
cleaner split.

**Why it goes one way.** A shell prompt is where you run programs, and reaching for Lua for one
quick thing is exactly what an escape is for. A Lua prompt is not the mirror of that — it is a
REPL, and `oslo.run{"ls", "-la"}` already runs a program from it. A second syntax for the same job
is a second thing to know and one more way for a line to mean something you did not type. So the
Lua side has **no prefixes at all**: every line is Lua, a leading `!` is a syntax error there
exactly as it is in any other Lua interpreter, and Shift+Tab is how you leave.

**What `!` shares with history.** `!!` is the most-typed two characters in any shell, so the line
between them is drawn where it can be drawn without guessing:

> History keeps the characters that **cannot begin a Lua expression**. Everything that can, is Lua.

`!!`, `!$`, `!^`, `!*` and `!?str?` stay history — Lua has no `!`, `$` or `?` at all, and no
expression opens with `^` or `*`. And `!5 + 5`, `!-x` and `!print(1)` are Lua. What that costs is
bash's numbered events `!5` and `!-2`, and `!name`; all three are ambiguous by construction, and
all three have the same better answer here in the history finder, which searches as you type and
shows what it found before it runs.

**The prefix is `$OSLO_LUA_PREFIX`**, one punctuation character, and that whole section is the
price of `!` alone — it is charged only when `!` is the prefix. Bash and oslo between them already
spend `= @ % : . # ~ $ > < & | * ? ^`, but `,` and `+` are claimed by neither and cannot begin a
Lua expression either, so setting one of those makes the rule "a leading `,` is Lua" with no
exceptions and hands every `!` form back to bash. `none` removes the escape; Shift+Tab still
switches. A value that is not a single punctuation character is ignored rather than half-honoured,
because a prompt that quietly reads a language other than the configured one is the exact failure
this design exists to prevent.

The prefix does not touch `current`, so the prompt comes back in the language it was already in.
The two languages share one namespace either way: a variable the session inherits is a Lua global,
and a global a config assigns is a shell variable (`tests/lua_mode_tests.rs`).

### What follows the language, and what does not

The full remembered set is `(line, language)` pairs in `oslo_ui::recall`, seeded at startup from
the command log — which keeps a mode column of `"sh"` or `"lua"` — and appended to as you type.

| behaviour | per language | how |
|---|---|---|
| ghost suggestion from history | yes | `recall::suggest` asks `prompt::language()` first |
| suggestion from the local store | yes | the `run` index is keyed `(dir, mode, argv)` |
| history finder (Up, Ctrl-R) | yes | rows filtered on `command.mode` |
| command-name ghost hint | shell only | `hinting.rs` answers `None` when the language is not `sh` |
| command-name completion | shell only | `completion.rs`, in command position only |
| path, variable and config completion | both | still meaningful in Lua, so not gated |
| repair (the "did you mean" line) | shell only | everything it can offer is a shell command |
| `!!` and friends | shell only, shell lines only | `recall::for_language` supplies the set |
| completeness check under `PS2` | yes | shell parser or Lua parser, by `reading` |
| `$HISTFILE` | no, deliberately | stays a flat file so anything that reads it still works |

There was once a `load_history_for` that cleared the editor's history and refilled it with one
language. It is gone, and both reasons matter: it **corrupted `$HISTFILE`** — the editor appends
the entries added since the last clear, so re-seeding N lines wrote all N again on the next command,
once at startup and again on every toggle — and it could not work anyway, since the language can
change mid-line from somewhere that cannot reach the editor.

### Completion, suggestions and the flat namespace

A Lua line is completed **as Lua and only as Lua**. Every shell answer — a command name, a path, a
`$variable`, an `@mark` — is either useless there or actively wrong: `$HO`+Tab used to produce
`$HOME`, which is a lexer error in the language being typed, while `pri`+Tab and `oslo.`+Tab
offered nothing at all.

What is offered instead comes from the session itself: the globals, the keys of the table being
indexed (`oslo.ma`→`oslo.math`, `os.tim`→`os.time`), the methods after a `:`, and Lua's reserved
words. Nothing is offered inside a string or a comment, where a name is not a name. The ghost
suggestion draws from the same source, and only when exactly one name matches — a hint is a promise
that the accept key gives you *that*.

The split is deliberate: working out **what is being typed** is text and lives in the editor;
working out **what names exist** needs the interpreter. Only names cross between them, never
values — `_G` is cyclic and deep-copying it to answer a Tab would be absurd.

**Every `oslo` member is also a global.** `fs`, `json`, `git`, `run`, `path` and the rest need no
prefix, and `oslo.fs` keeps working. Three rules keep that from taking anything away:

* a name already in `_G` is never replaced — `math` is Lua's;
* two tables under one name are **merged**, missing keys only, which is what makes `math` work:
  `math.floor` and `math.pi` stay exactly where they were and `math.eval`, `math.convert` and
  `math.session` join them. The two share no key, and a test checks that rather than assuming it;
* only tables and functions are lifted, so there is no bare `version` global holding a string.

## What makes it different

oslo's log keeps the language beside each line, in a mode column, because recalling a Lua line
while the prompt is in shell mode has to run it as Lua — a flat list of lines with nothing recorded
about what each one was cannot answer that. `$HISTFILE` is still written and still flat, so
anything that reads it keeps working.

`PS1` is honoured for shell lines and cannot win for Lua ones. It describes a shell prompt, and
drawing `oslo$` in front of something that is not a shell command is exactly the confusion the
language segment exists to stop.

The toggle is Shift-Tab because `BackTab` is the only key in the Tab family a terminal delivers
distinctly. Ctrl-Tab is indistinguishable from Tab in the legacy encoding every terminal still
falls back to, so binding it would silently do nothing on a plain tty.

## Configuration

The keys. There is no `$OSLO_TOGGLE_KEY`; bindings live in one table so there is no second place for
them to disagree from.

**Two keys switch it, and they fail in different places.** `Shift+Tab` has to be *reported* as
Shift+Tab, and a terminal that does not report the modifier — Alacritty without the kitty keyboard
protocol — leaves no way to change language at all. `Ctrl+Space` asks the terminal for nothing: it
is `NUL` on a plain tty and `CSI 32;5u` under the kitty protocol, and oslo decoded both to the same
key long before either was bound to anything. Its own weakness is that ibus and fcitx claim it as
the input-method switch, and an IME takes it before the terminal sees it. Hence two.

```lua
-- "toggle-mode" is the same action under another name.
oslo.keys["f2"] = "toggle-language"     -- another key as well
oslo.keys["shift-tab"] = "none"         -- and this turns a default off
oslo.keys["ctrl-space"] = "none"        -- as does this
```

**Tab twice on an empty line** is the third, and the one nothing can take away — a plain Tab is a
plain Tab everywhere. It is on by default, because the other two fail *silently* on a machine where
nothing looks wrong, and a fallback you have to go and find is not much of a fallback.

It applies only on an empty line, so Tab keeps its whole ordinary meaning the moment there is
anything to complete. What it costs is Tab at an empty prompt, which otherwise lists every name on
`$PATH`.

```sh
export OSLO_DOUBLE_TAB=off      # give the empty-prompt listing back
```

The language a session starts in. Both spellings reach the same shell variable.

```sh
export OSLO_DEFAULT_MODE=lua
```

```lua
oslo.opts.set("default_mode", "lua")
```

The character that runs one line as Lua from a shell prompt. One punctuation character, or `none`
for no escape at all.

```sh
export OSLO_LUA_PREFIX=,
```

```lua
oslo.opts.set("lua_prefix", ",")
```

What Enter does at a Lua prompt. `smart` — the default — sends a finished block and adds a line to
an unfinished one, which is what a Lua REPL usually does. `newline` always adds a line, and
**Ctrl+Enter or Alt+Enter sends**.

```sh
export OSLO_LUA_ENTER=newline
```

```lua
oslo.opts.set("lua_enter", "newline")
```

> **Alt+Enter is the one that always works.** Ctrl+Enter does not exist on a terminal without the
> kitty keyboard protocol: in the legacy encoding Ctrl-M *is* Enter, so the two cannot be told
> apart, and a prompt whose only send key was Ctrl+Enter would be a prompt that never ran anything.
> Alt+Enter is decoded in both encodings and mapped to the same action, so `newline` always has a
> way out of a block. That is also why `smart` is the default rather than the other way round.

Watching the switch. One hook covers vi-mode changes too, so a handler that cares about only one
reads `kind`.

```lua
-- m is { kind = "language", from = "sh", to = "lua" }
oslo.on.pre_mode_change(function(m)  end)
oslo.on.post_mode_change(function(m) end)
```

Drawing it. A prompt function is handed `language`, and `$OSLO_MODE` carries the same word for a
shell-side prompt.

```lua
oslo.prompt.left = function(p) return p.language .. " > " end
```

## Measurements

Both numbers are recorded in the source at the point they changed a decision.

| measured | result | where |
|---|---|---|
| one spawn of an external prompt program | 91 ms | `startup/read.rs` — why the mode-change redraw is not three extra renders |
| store suggestion keyed by directory + language | 33 µs | `recall/nearby.rs`, release, 25,000 rows / 3,000 directories |
| the same widened to the worktree | 1.8 ms for `cargo run --ex`, 7.1 ms for `c` | as above; 69 ms over one 26-character line, which is why it is memoised |

## What it cannot do

**Syntax colouring is not per language.** One shell highlighter paints both prompts; there is no
Lua lexer behind the editor. Nine reserved words overlap (`if`, `then`, `else`, `for`, `while`,
`until`, `do`, `in`, `function`), which is why a Lua line looks plausible rather than right:
`local`, `end` and `nil` get no colour of their own.

The plain Up/Down walk is **not** filtered by language. It reads the editor's own history, which is
the complete `$HISTFILE`-backed list. By default this never shows, because Up opens the history
finder and that filters on the mode column — but with `oslo.finder.enabled = false` the walk offers
lines from both languages.

The one-off prefix changes only how the line is run. While you are typing `!print(1)` at a shell
prompt the row still says `sh`, so the suggestion, completion and colouring are the shell's for
that line; only once it is accepted is it read as Lua. A Lua line is never examined at all, so on
that side what you see is always what runs.

Toggling part-way through an unfinished multi-line command switches the language the *rest* of that
command is read in, and the completeness check then asks the other parser about the whole buffer.
Nothing warns about this.

The toggle needs a terminal: with no tty, `read_line` reads a plain line off stdin and no key is
ever seen, which is why `tests/lua_mode_tests.rs` covers the language rules and `$OSLO_DEFAULT_MODE` but
not the key.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-runtime/src/startup/mode.rs` | `Mode`, `classify`, `starting_mode`, `TOGGLE_KEY` |
| `crates/oslo-runtime/src/startup/read.rs` | `read_command` — the loop that owns `current` and `reading`; `is_complete` |
| `crates/oslo-ui/src/edit/session.rs` | `Bound::ToggleLanguage`, `Step::ToggleLanguage`, `Outcome::ToggleLanguage`, `read_line` |
| `crates/oslo-ui/src/row.rs` | `note_row`, `language`, `repaint` — the process-wide prompt-row record |
| `crates/oslo-ui/src/recall/mod.rs` | `seed`, `remember`, `for_language`, `suggest` |
| `crates/oslo-ui/src/recall/nearby.rs` | `from_store` — the directory-and-language keyed query |
| `crates/oslo-runtime/src/startup/recall.rs` | `seed_history`, `remember_history` |
| `crates/oslo-runtime/src/startup/native.rs` | `ShellAssist::binding`, `open_finder` |
| `crates/oslo-ui/src/keys.rs` | `Action::ToggleLanguage`, `Action::Nothing` |
| `crates/oslo-ui/src/prompt.rs` | `LANGUAGES`, `measured_width`, `render_default_left_prompt` |
| `crates/oslo-runtime/src/startup/prompt.rs` | `primary_prompt`, `segment_context` |
| `tests/lua_mode_tests.rs` | the language rules, the shared namespace, per-language completeness |
