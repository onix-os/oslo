# oslo

A POSIX shell in Rust that also speaks Lua, with a structured pipeline that scripts written before
it existed provably cannot reach. Linux only.

```sh
oslo                       # a prompt
oslo script.sh arg1        # run a shell script
oslo build.lua             # a Lua script — same command, no flag
oslo -c 'echo hello'       # run a command
```

<!-- demo:begin -->
[![oslo](https://asciinema.org/a/1262743.svg)](https://asciinema.org/a/1262743)
<!-- demo:end -->

Rows on the pipe, two languages at one prompt, a shell that learns what you run — and knows what you
meant. Every feature has its own recording in [docs/features](docs/features/), each made by a script
in [scripts/demo](scripts/demo/) so it can be made again after the code changes.

---

## Two languages, one prompt

Shift+Tab switches between shell and Lua **in place** — your line, your cursor and your history stay
where they are.

```
bresilla@tron | I | sh  > ls -la                    ❮  (develop)  ~/src/oslo
bresilla@tron | I | lua > for _, f in ipairs(sh.ls(".")) do print(f.name) end
```

Each language keeps its own history, suggestions, completion and syntax colouring. A `=` prefix runs
one line as Lua from a shell prompt (`=print(("x"):rep(40))`); `!` does the reverse. Neither changes
the mode.

## Structured pipelines

A command produces two things: text for you, rows for the next command. The pipe decides which
**before anything runs**, by reading what each stage declares.

```
$ df | where 'free < 1e9' | sort-by free
filesystem  size  used  free   capacity  mounted
efivarfs    192K  169K  19K    91        /sys/firmware/efi/efivars
tmpfs       1.0M  0B    1.0M   0         /run/credentials/systemd-journald.service
```

Into a pipe, the same rows become plain complete text — no colour, no borders, no `4.2G` where a
program wanted `4509715660`. It works with programs that know nothing about oslo:

```sh
kubectl get pods -o json | from json | where 'status.phase == "Running"' | cols name
cat /etc/passwd | parse '{user}:{x}:{uid}:{rest}' | where 'uid > 1000' | get user
ps | where 'cpu > 10' | sort-by cpu | first 5 | to json | jq .
```

Filters are **Lua**, not a dialect invented for the occasion, so the escape hatch is the same
language as the filter: `ls | each 'print(name .. " is " .. size)'`.

Verbs: `where` `each` `cols` `get` `sort-by` `first` `final` `length` `to` `from` `lines` `parse`.
Summaries: `group-by` `count` `distinct` `stats`. Producers: `df` `ps` `ls`, and anything you register
yourself.

```sh
ps | group-by user | count            # how many processes each user has
ls | distinct kind                    # one of each, keeping the first
df | stats free                       # count, min, max, sum, mean over a column
```

Selection alone is a nicer `awk`; `ps | group-by user | count` is a query `ps | grep` cannot express.
There is no `join`: it needs a *second* input stream, and a pipeline is a line.

### Your POSIX scripts cannot reach any of it

Structure flows only between two stages that **both** declare they understand it, and every name
that can carry a declaration is one oslo invented. A script written before oslo existed cannot name
one, so every edge in it plans to bytes.

That is not a promise, it is a build failure: `tests/posix_stays_on_the_byte_path.rs` runs all 416
corpus scripts and requires zero structured edges. There is no new pipe operator either — `a |> b`
is already valid POSIX, so the operator would itself be the hazard. Design:
`docs/features/structured-pipelines.md`.

## The prompt

Named segments with priorities, so a narrow terminal drops the least important piece rather than
truncating mid-word:

```lua
oslo.theme.styles["my.dir"] = "fg:#8be9fd bold"

oslo.prompt.left = {
  oslo.segment{ name = "dir", priority = 90,
    render = function(ctx) return {{ text = oslo.path.shorten(ctx.cwd, 3), style = "my.dir" }} end },
}
```

`ctx` carries `status`, `ok`, `duration_ms`, `cwd`, `branch`, `vimode`, `language`, `cols`, `jobs`
— gathered once so five segments do not run `git` five times.

Or hand it to another program, with a deadline so a hung tool cannot hang your shell:

```lua
oslo.prompt.left = { command = "starship", args = { "prompt", "--status=$status" },
                     timeout_ms = 200, async = true }
```

Or just use `$PS1`, with the full escape set — `\u \h \w \$ \t \A \d \j \! \[ \]` and octal.

## Interactive

- **Ghost suggestions** from history, per language, never crossing between them, and answering for
  the directory you are standing in. Right takes the one on screen, Tab opens the dropdown when
  there is a choice to make
- **A completion dropdown** with columns, per-kind info and frecency ranking
- **Matching that is a transform, not a prefix test** — in the dropdown, `/u/s/b` reaches
  `/usr/share/bin`, `f-b` reaches `foo-bar` and `gco` reaches `git checkout`, each looser pass
  running only when the stricter one found nothing, so an exact match is never diluted
- **A full-screen history finder** on Up, seeded with whatever you had typed. Left and Right narrow
  and widen the scope — global, host, session, directory, workspace — and Delete forgets a command
  for good, after asking
- **First-class vi mode** on fish's model: cursor shape says the mode, the prompt says it too
- **Its own line editor** — buffer, layout, redraw, emacs and vi keymaps — so oslo owns the row it
  edits rather than renting it. No `readline`, no `rustyline`.
- **Grapheme-aware editing** — combining marks, emoji modifiers, flags, keycaps and ZWJ emoji move,
  delete, wrap and truncate as whole displayed characters
- **A right prompt**, drawn without the save/restore multiplexers fight over
- **Syntax highlighting** that marks a command that does not exist

### Abbreviations

```sh
abbr gco 'git checkout'      # typing `gco ` leaves `git checkout ` in the buffer
abbr gc 'git commit -m "%"'  # `%` says where the cursor lands
```

Better than an alias for a shell that promises not to change what scripts see: the real command
lands in the buffer, in history and in the log, and you watch it happen.

### Small named things, kept

```sh
oslo macros add --alias gs 'git status --short' --tag git
oslo macros add --abbrev gco 'git checkout'
oslo macros add --func mkcd            # opens $EDITOR
oslo macros add --script deploy        # opens $EDITOR — any language, with a shebang
oslo macros show                       # the manager, on the whole screen
```

**Alt+\\ opens it from the prompt**, beside Ctrl+\\ for the scratch finder — the same key with the
other modifier, for the other list of things you keep. `oslo.macros.key` moves it.

A database rather than a file to edit and re-source. Four kinds under one word: an alias, an
abbreviation, a function and a script. `oslo macros show` is the history finder's screen pointed at
them — type to filter, ← → for the tag, Tab between what you stored and what your config defines,
Enter to edit in `$EDITOR`, Delete to forget, Space to turn one off for this session and three
spaces to turn it off everywhere. A change is live in every running shell before its next prompt,
for the price of a `stat`.

Aliases and abbreviations reach an interactive shell when it starts; a function or a script is found
when you call it, **after `$PATH` has already failed**, so nothing on the system can be quietly
redefined. A stored script runs from an anonymous in-memory file — no temporary file is ever
written. `alias` in a script and `oslo.alias` in your config still work, and the database is applied
last, so it wins.

[Macros](docs/features/macros.md) has the whole of it.

### Arguments, declared in comments

```sh
#!/usr/bin/env oslo
# @option -t --tries <N>   how many times
# @arg    target!          where to
argc "$@"
echo "$argc_target, $argc_tries times"
```

[argc](https://github.com/sigoden/argc)'s declaration language, vendored and parsed by the shell
itself: `--help` is generated, a bad flag is reported, and `deploy --<Tab>` completes from the same
comments with no completion file to install. A bash script gets the same parser through
`eval "$(oslo --argc-eval "$0" "$@")"` — oslo is a drop-in for the `argc` binary.

Behind the `argc` feature. [The whole of it](docs/features/argc.md).

### Keys that run your code

```lua
oslo.keys["ctrl-s"] = function(line) return "sudo " .. line.text end
oslo.keys["alt-u"]  = function(line)
  return { text = line.text .. " [" .. line.word .. "]", cursor = 0 }
end
oslo.keys["f2"] = "toggle-language"
```

Data in, data out — the handler is told about the line and answers with what it should become.
`f1` through `f12` work with both conventional terminal sequences and Kitty's disambiguated input.

### Where you have been

```sh
cd -         # back one
cd -3        # three back
cd root      # the top of the git worktree, found without running git
cd oslo      # somewhere you have been, best match wins
dirh         # the ring, and the numbers `cd -N` takes
```

`cd -` is a one-deep toggle, useless three wrong turns out, so every move is recorded and `cd -N`
reaches any of them. `pushd`/`popd` are untouched — scripts depend on them.

**POSIX first, always.** A real directory, a `-`, a `-P` resolve exactly as before; the frecency
jump is reached only once `cd` has already failed to find the argument as a path, and only in an
interactive shell.

Ranking is not zoxide's. Its four loudest bugs are one defect — keywords filter but take no part in
the score, so the most-visited candidate wins however badly it matched. Here *match quality is the
primary key* and frecency only orders equal matches, so `cd rust` cannot land in `prust`. Each of
the four is a named passing test.

### Navigate the filesystem

```sh
nav              # start here
nav /var/log     # start somewhere else
```

`nav` is a centered, history-styled filesystem navigator. Typing starts filtering immediately;
Up and Down move through the matches, Right or Enter opens a directory, and Left goes to its
parent. Delete asks first, then removes through oslo's own `rm` builtin, including its trash
settings. The key legend is hidden until `?` toggles it. Esc changes the shell to the directory
on screen; Ctrl-C cancels without moving it.

### What it remembers

A second database, beside the history one, recording where you go and what you run there:

| | |
|---|---|
| `dir` | path, visits, last visit, dwell time, git worktree root |
| `run` | the line, the directory, exit status, how many times, total and worst duration |

Which makes the ghost suggestion answer differently depending on where you are standing:

```sh
~/work/alpha > cargo run --ex⏎     # → cargo run --example xyz
~/work/beta  > cargo run --ex⏎     # → cargo run --example abc
```

The exact directory is asked first, then the worktree, then flat history. A line that never once
succeeded is never offered, so a typo stops haunting you.

It is `0600`, tightened before the first statement so the write-ahead log is born private too. A
line beginning with a space is not recorded, secrets are reduced to the command name, and **a
non-interactive shell never opens it at all**. `history -c` clears the lines and keeps the
directories: "forget what I typed" is not "forget where I work".

No daemon. The read is a B-tree range scan rather than a `LIKE` — 13 µs against 25,000 rows — which
is what makes a cache unnecessary, and a cache stale between two terminals impossible.

### Colours

Every one of the 54 roles — 23 syntax, 19 dropdown, 7 prompt, 5 widget — is settable, and each
takes an index or an RGB triplet:

```lua
oslo.theme = {
  syntax = { command = "#7cff9d", keyword = "212", comment = { fg = "244", italic = true } },
  pager  = { bg = "#101010", sel_bg = "238" },
  prompt = { cwd = "blue", git = "green" },
  ui     = { accent = "213" },
}
```

That "every one" is a test, not a claim: adding a role to the theme without a reader fails
`every_role_can_be_set_from_a_config`.

**oslo brightens its own colours and never yours.** The syntax palette is absolute RGB, so it is
lifted on HSV's value and saturation axes to sit off the background. An ANSI slot is returned
untouched — `"green"` means "colour 2, whatever this terminal thinks that is", and a tool that
remaps the slots (pywal and friends) owns that answer. Near-greys are left alone too, which is what
keeps the dropdown's chrome and the black on `sudo`'s red background from being "brightened".

HSV rather than HSL, because in HSL lifting an already-light colour bleeds it toward white:
`#ff5555` becomes `#ff8888`, a *paler* red. Brighter has to mean more colour.

### Fuzzy matching

```lua
oslo.completion = { fuzzy = "smart" }   -- off / tight / smart / loose
```

`off`, `tight`, `smart` or `loose` — how far your letters may scatter, capped at 1, 4 and 8
unmatched characters. `gco` reaches `git checkout` at `smart`, not at `tight`. Always the last pass,
so switching it on cannot push a candidate you actually prefixed down the list.

**In the dropdown only.** The inline ghost suggestion stays a strict continuation of what you typed,
because that is the only thing it can honestly be: the editor draws a hint as text appended after
the cursor, so a suggestion that *replaces* your line cannot be shown as one without lying about
what pressing a key will do.

## What you were about to type

**Behind `--features vista`, and off by default.** A default build suggests from history,
completions and `$PATH` — everything below this line needs the flag, which costs 341 KB:

```sh
cargo build --release --features vista
```

```lua
oslo.suggest.sources = { "predict", "history", "path" }
```

`predict` is a model of what *this* shell does, learned from the commands you have actually run and
kept as a small file beside the history. It reads in a tenth of a millisecond at startup and is
written once on the way out, so it costs the prompt nothing. A session that keeps no history
(`HISTFILE=""`) neither reads it nor writes it, and `oslo history clear` deletes it.

## What you probably meant

```
$ systemclt status -> [systemctl] status
```

A line that looks mistyped is answered **before you run it**, and Right takes the correction. It is
a different claim from the ghost suggestion and so it is drawn differently — one is text you might
be about to have, the other is the shell disagreeing with text you already have. They never appear
at once, and the same key accepts whichever is showing.

**Only the words that changed are bracketed**, in the ghost's own colour reversed; the arrow and the
words that were already right stay in the plain ghost grey. A correction shown whole makes you
re-read the line to find the one character that moved.

Two things know what you meant, and both are asked:

- **`$PATH`**, for the command word. `lsvlk` is a misspelling of a real program whether or not it
  has ever been typed here, so this works on a shell with no history at all.
- **the model**, for the rest of the line. `echo hello wrold` needs something that has watched you
  work, and only a proposal close enough to be a *retyping* is offered — a different command is a
  prediction, not a correction.

### And the one that already ran

```
$ git stauts --short
$ <F4>
$ git status --short
```

The correction you actually want is usually of a command you have **already run and watched fail** —
by then it is not on the line to be fixed. `oslo.repair()` with no argument answers for that one:

```lua
oslo.keys["f4"] = function(line)
  if line.text == "" then return oslo.repair() or "" end   -- the command that just failed
  return oslo.repair(line.text) or line.text               -- the one being typed
end
oslo.theme = { syntax = { repair = { fg = "yellow", reverse = true } } }  -- follows the ghost otherwise
```

It lands in the editor like everything else here — oslo never re-runs a command for you.

**Nothing here runs anything.** The correction lands on your input line and Enter is still yours,
which is what makes a wrong guess cost a keystroke instead of a command. There are no rules to
maintain either: a repair can only ever be built out of `$PATH` and commands you have really run.

**Only a command that worked is learned.** A mistyped line inside the model is a command like any
other, and repair for it goes quiet — which would break the case above, since you ask *after* the
failure. It also means a typo is never suggested back to you.

`oslo.predict.next(partial, n)` and `oslo.predict.repair(line, n)` ask the model directly, and both
answer a list of `{ line, probability }`.

## Your own tools

```lua
oslo.register_tool{ name = "hosts", produces = "rows",
  rows = function(argv) return { { host = "alpha", ip = "10.0.0.1" } } end }
```

```
$ hosts | where 'ip:match("^10%.")' | cols host port
```

A tool says what its rows *are*. The shell decides how they are drawn — and when the next stage
wants rows, nothing is drawn at all.

### Work that happens off the prompt

```lua
oslo.spawn{ "git", "status", "--porcelain",
  on_exit = function(out, status) oslo.state.set("git.dirty", out ~= "") end }
```

**The callback arrives between commands**, not the instant the process exits — the same safe point
timers fire at, where the shell holds nothing and can call Lua. That is the honest limitation: a
prompt segment reading `oslo.state` shows the answer from a moment ago instead of blocking the draw
to fetch a fresh one. One process, one callback; there is no scheduler and no promise. A missing
command answers 127 and a `timeout` answers 124, which are the statuses a shell already uses for
both.

### Completion you declare rather than compute

```lua
oslo.completion.spec {
  command = "notes",
  subcommands = { { name = "new", desc = "start one" },
                  { name = "list", desc = "every note",
                    flags = { { "--since", desc = "only newer than" } } } },
}
```

The same shape the specs for `git`, `cargo`, `docker` and `npm` are written in, and it runs through
the same code at Tab time — nested subcommands, flags scoped to the subcommand you are in,
descriptions in the dropdown. `oslo.completion.for_command` is still there for the cases that have to
look at the machine; a spec answers the shape of a command, a function answers what is on it.

### Running commands from Lua

Two forms, and the difference between them is pathname expansion.

```lua
sh.rm("*.txt")                             -- the pattern is read, as at a prompt
oslo.run{ "rm", name }                     -- `name` is one filename, whatever is in it
oslo.run{ "rm", "*.txt", glob = true }     -- ask for it explicitly
```

`sh.cmd(…)` is spelled to stand in for a command line, so it expands patterns like one. A pattern
matching nothing is passed through unchanged — POSIX, and what every shell without `nullglob` does
— which is also what lets `sh.printf("[%s]", x)` through untouched.

`oslo.run{…}` is the exact-argv form: the list you write is the list that runs, with no quoting
step and so no quoting bug. That is what makes it safe for a filename the script did not choose,
and it is the escape when an argument must not be read as a pattern. The command word is never
expanded in either form.

`oslo.glob(pattern)` answers the question directly, and returns an empty table rather than the
pattern when nothing matched.

## The terminal knows what is happening

Every interactive command has one balanced semantic lifecycle. Portable terminals receive OSC 133
`A/B/C/D` boundaries with one stable shell-session ID; blank, cancelled, and EOF input closes
without pretending that a command ran. OSC 7 publishes the working directory, OSC 0 sets the title,
OSC 8 carries Oslo-owned hyperlinks, and OSC 52 powers the `copy` builtin over SSH. OSC 52 can still
be refused by the terminal's clipboard policy.

### A shell inside a shell says so

```text
You are already in oslo. Start a nested shell? (one deep)

  Nested shell   Stay here

←→ choose • y/n answer • enter confirm
```

Typing `oslo` at an oslo prompt has always worked and never said anything, which is the problem: the
new shell looks exactly like the old one, so the usual way to find out you are two deep is an `exit`
later that does not close the terminal. Enter answers "stay here", because that is the answer you
cannot regret.

`$OSLO_NESTED` counts, for a prompt that wants to show it — `0` in a fresh terminal, `1` inside one
oslo, and so on. Every shell publishes it, `-c` and scripts included; only an interactive one asks.

```sh
[ "${OSLO_NESTED:-0}" -gt 0 ] && printf '⧉%s ' "$OSLO_NESTED"
```

**One terminal, one stack.** A tmux pane, a hexe pod and an ssh login all inherit the count from the
shell that opened them and none of them is inside it — each runs its shell on a pty of its own, and
there is nothing there to `exit` back into. So a count only counts when two things hold: it was set
on *this* terminal, and the shell that set it is **still a live ancestor** of this one. A server that
has been running for a week hands out the environment it started with, so the second half is what a
stale variable cannot survive. Anything else starts again at `0`.

Asked only when there is a terminal to ask on — `command | oslo -i` is a shell with no person at
the other end of stdin, and a question there would be answered by the script's first line.
`oslo.misc.nested_ask = false` turns it off for somebody who nests on purpose; the count stays.

### Copying what a command printed

```sh
ls | copy                  # a pipe, arguments, or a file: copy hi / copy < notes.txt
keep git log --oneline     # run it, watch it, and keep what it printed
copy --last                # that, on the clipboard
keep -e make build         # keep the errors too
```

`copy` reaches the clipboard through OSC 52, so it works over SSH and in a container with no
`xsel` — and the terminal may refuse it, in which case nothing arrives and there is no reply for
oslo to read.

**`keep` is a prefix because output is gone once it is printed.** To have a copy the shell must
stand between the command and the terminal, and standing there for *every* command means holding
the largest thing you ever run in memory and turning `isatty` false for all of them. So it is one
command at a time. You still watch it scroll: every chunk is written to the terminal before it is
kept. What it costs is that a program which colours only for a terminal will not colour here — and
what is left of an escape sequence is taken out before it is stored, because a clipboard full of
`\x1b[32m` is not the output.

Kept per session, so two terminals do not answer for each other, in a file rather than in memory —
`keep` inside a pipeline runs in a forked child, and `copy --last` still finds it. A capture over
1 MiB keeps its tail and says so, since the end of a build log is the part worth having.

A key, if you want one — no new code, the existing handler rewrites the line:

```lua
oslo.keys["alt-enter"] = function(line) return "keep " .. line.text end
```

### Asking for something, from any shell

```sh
ui choose alpha beta gamma          # at an oslo prompt: the builtin
oslo userin choose alpha beta       # from bash, a Makefile, a .desktop file: the same widgets
```

Thirteen of them — `input`, `write`, `confirm`, `choose`, `filter`, `table`, `file`, `style`,
`format`, `join`, `pager`, `log`, `spin` — and `oslo userin --help` lists them with their options.

**Two doors, one body.** `ui` is a builtin, and a builtin cannot be reached from bash: a script, an
`sh -c`, a status bar reach a *program*. `oslo userin` is that program, running the same code, so
the two can never disagree about what a widget does. It is why a shell that ships its own prompts
can lend them to everything else on the machine, with nothing installed beside it.

Three rules a script depends on:

- **the answer is stdout, everything else is stderr**, so `$(oslo userin input)` captures the answer
  and the widget still draws on the terminal;
- **cancelling is status 1 with no output**, so `x=$(oslo userin input) || exit` is right — a widget
  that returned `""` on Esc would make cancelled and empty the same thing;
- **no terminal is status 2**, distinct from cancelled, so a script can tell "nobody was there to
  ask" from "they said no".

Items come from the operands or from stdin, so `ls | oslo userin filter` and
`oslo userin filter a b c` are both the obvious thing.

[Asking for something](docs/features/userin.md) has the whole of it, including the third door:
`oslo.ui.choose{…}` from Lua, the same widgets again.

The native editor enables bracketed paste while it owns the line. A pasted newline is inserted as
text and does not execute until Enter is pressed. Pasted and typed control characters stay exact in
the command buffer but redraw as inert notation such as `^[`, `^I`, `^M`, and `^?`; raw OSC and CSI
bytes never reach an editor frame. Cursor movement, deletion, wrapping, completion width and click
placement operate on extended grapheme clusters.

Startup sends one ordered, bounded query batch for the background, Kitty keyboard support,
synchronized output and OSC 99 notifications, with primary device attributes as the final barrier.
The whole exchange has one 100 ms deadline. Replies and total input are size-bounded, and bytes
typed during the exchange are handed to the editor in their original order. When Kitty keyboard
support is verified, Oslo pushes disambiguation mode for the editor and restores it before command
execution. Completion uses the same decoded event stream as the editor, so CSI-u keys, paste, focus,
mouse, resize and the character that dismisses the menu are not split or lost.

Terminal-specific behavior is selected conservatively:

| Host | Integration |
|---|---|
| Any interactive terminal | OSC 133 lifecycle, OSC 7/8/52, bracketed paste |
| VS Code (`TERM_PROGRAM=vscode`) | One OSC 633 `A/B/E/C/D` lifecycle plus rich CWD detection |
| iTerm2 (`TERM_PROGRAM=iTerm.app`) | User-variable encoder and foreground OSC 9;4 progress |
| WezTerm (`TERM_PROGRAM=WezTerm`) | User-variable encoder |
| Kitty-keyboard compatible terminal | Queried CSI-u disambiguation with push/pop restoration |
| `TERM=dumb`, scripts, and `-c` | No semantic marks or terminal queries |

Features with no reliable discovery remain exact opt-ins: `OSLO_TERMINAL_EXTENSIONS=kitty` enables
the Kitty OSC 133 continuation marker and percent-encoded command metadata, while
`OSLO_SYNC_OUTPUT=1` forces DEC 2026 synchronized frames even without a verified reply.
`OSLO_CLICK_EVENTS=1` enables OSC 133 prompt-scoped clicks through `cl=line` and `click_events=2`;
it does not capture the global mouse or query the cursor after a click. The separate
`OSLO_CLICK_EVENTS=legacy` fallback enables DECSET 1000/1006 only while the line editor owns the
terminal and accepts the standard two-coordinate DECXCPR response. Other values enable neither
click path.

Exact command metadata is taken after `pre-cmd` replacement. Leading-space/private commands keep
their lifecycle but publish no command text. OSC and OSC 633 fields escape separators and control
bytes. The VS Code adapter emits no nonce because Oslo receives no documented launch nonce.
Slow-command notices emit one rich OSC 99 title/body transaction when support is verified,
requesting delivery only while unfocused when the terminal advertises that occasion. Otherwise
they emit one sanitized OSC 777 fallback, never both.

The `marks` feature controls semantic boundaries, titles, working-directory reports and
terminal-specific adapters as one unit. Turning it off leaves the prompt and editor functional but
emits none of those sequences. OSC 52 is separate because `copy` is an explicit user action.

`status terminal` prints the immutable capability snapshot and the origin of each selection without
running a query. Scripts and `-c` report a disabled snapshot and emit no terminal escapes.

Terminal metadata is visible to the terminal process: command text is not encrypted or hidden from
the emulator. Separators and control bytes are encoded or removed before a field is emitted, and a
private leading-space command omits its text entirely. No nonce is invented or treated as
authentication. Each reply is limited to 4096 bytes, the batch is limited to 16384 input bytes, and
unrelated input is preserved.

Compatibility evidence recorded on 2026-08-07:

| Terminal | Version | Evidence | Result |
|---|---:|---|---|
| Kitty | 0.48.2 | Real PTY on headless Wayland: prompt/output navigation, multiline PS2, finder restore, click editing, F2 binding and verified Kitty/sync/OSC 99 status | Pass |
| Ghostty | 1.3.1 | Real PTY on Xvfb: multiline/right prompt, screen extraction and click editing | Pass |
| WezTerm | 0-unstable-2026-07-16 | Real PTY on headless Wayland: multiline transcript, OSC 7 CWD update and no leaked replies | Pass |
| iTerm2 | macOS-only; unavailable on this Linux host | OSC 133, metadata and nested-session PTY acceptance tests | Protocol pass; GUI not run |
| VS Code | unavailable on this host | `TERM_PROGRAM=vscode` PTY acceptance test verifies one OSC 633 lifecycle | Protocol pass; GUI not run |
| tmux | 3.6 | Real nested PTY: balanced command block and no leaked query reply | Pass |
| GNU screen | 5.0.2 | Real nested PTY: balanced EOF closure and no leaked query reply | Pass |
| Basic terminal | `TERM=dumb` | PTY acceptance test verifies readable editing with no semantic bytes | Pass |

## POSIX, where it counts

The language is the real thing: pipelines, redirections including heredocs and here-strings, all the
control flow, functions, `${var:-d}` and the rest of parameter expansion, arithmetic, globbing,
field splitting, job control with proper process groups and `tcsetpgrp`.

Correctness is measured rather than asserted. 416 scripts in `tests/corpus` run under both oslo and
bash and are compared byte for byte, with known differences listed in
`tests/differential/expected_fail.rs` as a two-way ratchet — the suite fails if a listed case starts
passing, so a stale entry cannot survive. Unit and integration tests run alongside it.

## Directory environments

A `.env.lua` or a `.envrc` in a project, found by walking up from where you are, nearest ancestor
wins. **Leaving puts everything back** — including variables the file unset, and variables that had
no value before and end up with none again rather than with an empty one.

```lua
-- .env.lua
oslo.env.set("DATABASE_URL", "postgres://localhost/app_dev")
oslo.env.set_alias("t", "cargo test")
oslo.direnv.path_add("./bin")   -- prepended, idempotent, gone when you leave
oslo.ui.prompt(function() return "PRODUCTION> " end)
```

Variables, aliases, `PATH` and the prompt all come back on the way out — one that was shell-local
before the directory exported it comes back *local*, not deleted. Keybindings deliberately do not:
a key meaning different things in different directories, with nothing on screen to say so, is worse
than not having the feature.

```sh
direnv allow      # trust this file, as it stands right now
direnv deny       # refuse this path, whatever it becomes
direnv status     # what is loaded, what was found, whether it is trusted
```

Nothing is read until allowed, because `git clone` then `cd` is otherwise arbitrary code execution.
Allowing hashes the file's **contents**, so editing it revokes the allowance; denying hashes only
the **path**, and survives every edit. Both take effect where you stand, not on the next `cd`.

```lua
-- a Nix flake's dev shell, without entering one
oslo.direnv.nix_develop()
```

direnv's `use flake` is `eval "$(nix print-dev-env …)"` — 100KB of generated bash. This reads
`--json` instead. Why it is built in rather than left as a recipe: the two forms do **not** carry
the same variables. `nix` withholds `HOME` and four others from the shell form because setting them
would wreck the shell you are in — `HOME` in a derivation is `/homeless-shelter` — and `--json`
applies no such filter.

**`.envrc` too, with direnv's stdlib.** A project's `.envrc` is checked in and shared and is not
yours to convert, so oslo reads it — as shell, in this shell, with `PATH_add`, `use flake`,
`layout python`, `dotenv`, `source_up`, `watch_file` and the rest reimplemented in Rust rather than
shipped as 1,400 lines of bash. Your `~/.config/direnv/direnvrc` and `direnv/lib/*.sh` are sourced
first, so the `use_` and `layout_` functions you already wrote are there when a project calls them.

```sh
# .envrc — works as written, no conversion
use flake
PATH_add ./bin
dotenv_if_exists .env
watch_file schema.sql
```

A directory with both is governed by `.env.lua`: a repository that has both almost always has the
`.envrc` for everyone else, and running both would apply one environment twice. The stdlib exists
while an `.envrc` is loading and nowhere else — `PATH_add` at the prompt would edit an environment
no file is holding open. Either way there is no bash subprocess and no serialised diff in your
environment beyond the undo record: oslo *is* the shell.

Because it is Lua, a directory can set more than variables — an alias for its test command, say —
and **aliases are restored on the way out too**, so a project's `t` cannot follow you into the next
one and run the wrong tests. Whatever the file prints is grouped under it, repeats collapsed with a
count.

## nix, as data

Behind `--features nix`, and independent of `direnv`. Everything `nix` answers as JSON, as ordinary
Lua tables:

```lua
oslo.nix.run{"flake", "metadata"}    -- any nix command; the only part written in Rust
oslo.nix.inputs()                    -- every input, with how old its pin is
oslo.nix.shells()                    -- the dev shells this machine can enter
oslo.nix.dirty()                     -- has the flake uncommitted changes
```

**There is no `oslo nix` subcommand and will not be.** The feature is one generic call plus names
written in Lua, so a plugin adding `closure_size` is a Lua file rather than a patch to the shell,
and any helper can be replaced by assigning over it.

Why generic: twenty-three subcommands advertise `--json` and nix says its interface is unstable —
and the help text is not reliable, since `nix registry list --help` documents a `--json` that the
command then rejects. A wrapper per command would have shipped one that cannot work.

`oslo.nix.inputs()` is the one that says something you cannot already see, and it reads the lock
rather than evaluating anything:

```
systems       github    1220 days
flake-utils   github     636 days
nixpkgs       github     125 days
```

Nothing runs on its own. Completion for the real `nix` binary is opt-in, in one line:

```lua
oslo.completion.for_command.nix = oslo.nix.complete
```

`docs/features/nix.md` has the rest.

## One shell, several histories

```sh
oslo                            # ~/.local/share/oslo/history/default/
OSLO_PROFILE=claude oslo        # history/claude/ instead — the default untouched
```

A name is a **letter, then letters, digits, `_` or `-`** — anything else is refused rather than
cleaned up, because the name is the file and a typo must not quietly write somewhere else. An
unusable name falls back to `default` and says so once.

**`$OSLO_PROFILE` and nothing else.** There was a `--profile` flag; it is gone. A profile is a
property of a session, not of one command: export it once and every shell anything spawns inherits
it, which is the point when the thing spawning them is an agent running thousands of commands. A
flag only covers the invocation you remembered to put it on.

The store is named after a **profile**, `default` unless you say otherwise. Agents shell out constantly,
and every line they run otherwise lands in the history you are trying to search and in the frecency
table that decides what `cd` and Tab suggest. Give them a profile and that stops.

It is a name, not a lock: two shells can share one, and **Tab in the history finder moves to the
next profile** — which is how you go and read what the agent ran without leaving your shell.

A profile is a **directory** — `~/.local/share/oslo/history/<name>/` holding `hist.db`, `hist.lock`
and `hist.model`. So renaming one is `mv`, deleting one is `rm -r`, and copying one to another
machine is `scp -r`; none of those were quite right when the three files sat flat beside each other.

There used to be two files, `history.db` and `track.kv`. There is one now. Nothing migrates that
older pair — delete them.

**`$OSLO_SESSION`** names the session itself, and is exported. A shell sets it once; everything the
shell starts — a subshell, a tool, `oslo macros` — reports the session it is part of rather than
inventing one of its own, which is what lets a child process say "this shell" and be believed.

## Tools

`oslo --help` lists them — `config`, `profile`, `history`, `aliases`, `hook`, and whichever of
`direnv`, `plugin` and `scratch` this build has:

```sh
oslo history
oslo config
```

**A script always wins.** The operand slot belongs to scripts — POSIX defines the shell as
`sh [options] [command_file [argument...]]` — so a word is only read as a tool when all three hold:

- it contains no `/`
- **no file of that name exists**
- it is one of those names

The second is what makes this safe rather than merely unlikely to bite. oslo does not search
`$PATH` for a script operand, so when no such file exists the alternative was never "run something
else", it was `No such file or directory`. Nothing that works today can change meaning; an error
becomes useful. `oslo ./history` and `oslo -- history` say "this is a path" and are honoured.

The slash rule is what keeps shebangs working. A script starting `#!/bin/oslo` is always run by
the kernel with a *slashed* argv[1] — `./history` from the current directory, the full path when
found on `$PATH` — so a bare `history` can only ever have been typed by a person.

## Configuration

`~/.config/oslo/config.lua`. One file, one language, one place — there is no shell-syntax config.

What a shell reads before its first command, and when:

| | |
|---|---|
| **login** (`-l`, or `argv[0]` starting `-`) | `/etc/profile`, then `~/.profile` |
| **interactive** | `~/.config/oslo/config.lua` |
| **any** | `$ENV`, last, since `~/.profile` is where it is usually set |

The profile files are POSIX's and shell syntax, and oslo is a `/bin/sh` before it is anything else
— it does not get to refuse them. `/etc/profile.d` is **not** walked separately: `/etc/profile`
does that itself with `run-parts`, and a shell that also did it would source every file twice.
Root needs no special case, because `~/.profile` follows `$HOME`.

To read a shell file from the Lua config — an `aliases.sh` shared with your other shells — use
`oslo.source`, which runs it in *this* shell so its aliases and functions stick.

```lua
oslo.completion.max_rows = 12
oslo.suggest.accept = "ctrl-f" -- as well as Right, which always accepts
oslo.vi.enabled = true
oslo.source(oslo.env.get("HOME") .. "/.profile") -- shell files, sourced into *this* shell
oslo.on.cd(function(dir) print("now in " .. dir) end)
oslo.on["command-not-found"](function(name) print(name .. " is not installed") end)
```

Anything in `~/.config/oslo/conf.d/*.lua` runs first, in name order, and `config.lua` runs last.
That directory is fish's, and it is there for the same reason: a plugin or a dotfile repo needs
somewhere to add a line without editing a file it does not own, and the file you wrote by hand
keeps the final say.

### The settings

Shown with their defaults, so the options are visible without reading the source. Every line here
is what oslo already does — you only need the ones you want to change.

```lua
oslo.misc.welcome       = true        -- the startup banner
oslo.misc.greeting      = nil         -- a line of your own instead of the banner
oslo.misc.escape_delay  = 25          -- ms to wait for the rest of an escape sequence; raise on ssh
oslo.misc.color_depth   = nil         -- truecolor / 256 / 16 / none, when detection is wrong
oslo.misc.nested_ask    = true        -- ask before starting an oslo inside an oslo
oslo.misc.warnings      = true        -- the hint and warning boxes under `--help`

oslo.vi.enabled         = false       -- vi mode; true for vi, false for emacs only
oslo.vi.cursor_insert   = "line"      -- block / line / underscore, each + " blink"
oslo.vi.cursor_normal   = "block"
oslo.vi.cursor_replace  = "underscore"
oslo.completion.fuzzy   = "smart"     -- off / tight / smart / loose
oslo.completion.max_rows = 15

oslo.finder.enabled        = true     -- the full-screen history search
oslo.finder.key            = "up"
oslo.finder.limit          = 5000     -- distinct commands loaded when it opens
oslo.finder.confirm_delete = true     -- Delete asks before forgetting a command

oslo.macros.key            = "alt-\\" -- the macro manager, beside the scratch finder's ctrl-\

oslo.history.ignore     = {}          -- $HISTIGNORE patterns, matched against the whole line
oslo.history.ignore_space = true      -- a line starting with a space is not remembered
oslo.history.ignore_dups = false
oslo.notify.after       = 10          -- seconds a command must run to be worth a notification
oslo.notify.command     = nil         -- e.g. "notify-send {title} {body}", instead of the escape

oslo.abbr.gco = "git checkout"
oslo.abbr.brc = { "~/.config/oslo/config.lua", anywhere = true }

oslo.builtin.rm.to_tmp     = false    -- move removals aside instead of destroying them
oslo.builtin.rm.max_to_tmp = 100      -- MB; anything larger is destroyed
oslo.builtin.rm.trash      = "/tmp"

oslo.builtin.nav.fullscreen = true       -- alternate screen; false draws inline
oslo.builtin.nav.position   = "center"   -- top / center / bottom
oslo.builtin.nav.width      = 0          -- 0 uses the middle half of the terminal
oslo.builtin.nav.height     = 0          -- 0 uses the middle half of a full screen
oslo.builtin.nav.border     = "none"     -- none / rounded / square / double / thick
oslo.builtin.nav.border_fg  = nil
oslo.builtin.nav.border_fit = "content"  -- content / full
oslo.builtin.nav.legend     = false      -- ? toggles it while nav is open
oslo.builtin.nav.legend_gap = 1
oslo.builtin.nav.padding_x  = 1
oslo.builtin.nav.padding_y  = 0
oslo.builtin.nav.hidden     = false
oslo.builtin.nav.filter_at  = "bottom"   -- top / bottom
oslo.builtin.nav.reverse    = false
oslo.builtin.nav.scanner    = true
```

### `rm`

`rm` is a builtin, and at a prompt it is a friendlier one: it removes a directory without needing
`-r`, and with `to_tmp` on it *moves* what you delete instead of unlinking it.

```
~/p > rm -v build notes.txt
moved 'build' to '/tmp/build'
moved 'notes.txt' to '/tmp/notes.txt'
```

**In a script it is none of those things.** A builtin shadows `/bin/rm` for everything the shell
runs, and oslo means to be `/bin/sh` — so the extensions are confined to an interactive shell.
A `#!/bin/sh` file gets POSIX `rm`: a directory without `-r` is an error, and what is removed is
gone. `-s`/`--strict` asks for the same at a prompt. An option oslo does not implement —
`--one-file-system`, `-I` — is handed to the real `rm` on `$PATH`, so the builtin can never be less
capable than the system's.

`max_to_tmp` is there because the trash is usually on another filesystem, and `/tmp` is usually
tmpfs. Across a filesystem a move is a *copy*, so trashing a 4 GB file would copy 4 GB and then
hold it in RAM until the next reboot. Under the cap that cost is not worth noticing; over it, the
file is destroyed as `rm` has always destroyed things. A name already in the trash is never
overwritten — the second `notes.txt` becomes `notes.txt.1`.

### `\command` — getting past the shell's own version of a name

A builtin `rm` needs a short way to ask for the one on `$PATH`. `command rm` is not it: `command`
bypasses *functions*, and the builtin still wins.

| written | alias | function | builtin | runs |
|---|---|---|---|---|
| `rm` | expanded | used | used | oslo's |
| `\rm` | skipped | skipped | skipped | `/usr/bin/rm` |
| `\\rm` | expanded | used | skipped | the alias's target, unbuiltin |

`\cmd` reads as "whatever this shell has done to that name, give me the program". `\\cmd` is the
narrow one, for when the alias is the point — `alias rm='rm -i'` — and the builtin is not.

**Only at a prompt.** In a script a leading backslash does what POSIX says and nothing else: it
suppresses the alias, and then ordinary command search finds the function or the builtin as it
always has. `\\cmd` there is a command whose name begins with a backslash, and is not found —
which is what bash and dash answer, and why giving it a meaning at a prompt breaks nothing.
`tests/corpus/command_word_backslash.sh` is the case that pins both halves against bash.

Quoting is not escaping: `"rm"` and `'rm'` run the builtin, in oslo as in every other shell.

### `which` and `whereis` — builtins, because the programs cannot see a shell

```sh
which ls        # /usr/bin/ls
which ll        # ll: aliased to ls -alF
which cd        # cd: shell built-in command
which deploy    # deploy: stored script
whereis ls      # ls: /usr/bin/ls /usr/share/man/man1/ls.1.gz
whereis cd      # cd: shell built-in command
whereis deploy  # deploy: stored script
```

`/usr/bin/which` reads `$PATH`, and everything interesting about a name here is invisible to a
program: an alias is in this shell's memory, a builtin has no file, a stored macro is a database row
found after `$PATH`. So it answers "nothing" — not because nothing runs, but because it was asked by
something that cannot see. zsh made `which` a builtin for this reason; this is the same answer in
the same words.

A path prints bare, so `$(which foo)` is still a path. Anything else prints `name: what it is`,
which no script can mistake for one. `which --skip-alias` does the plain `$PATH` search the program
does, `\which` runs the program itself, and `which`, `type` and `command -v` all read the same
resolution order — a shell where those three disagree has more than one dispatch table.

**Only at a prompt**, like the frecency jump in `cd`. In a script both hand straight over to the
programs on `$PATH`, because `which` is not POSIX and somebody's configure script doing
`ECHO=$(which echo)` has to keep getting a path rather than a sentence about a builtin. What a
script has always had is `command -v`, which is POSIX, answers about *this* shell, and knows every
alias, builtin and stored macro there is. Where the system has no such program — a small
distribution may ship neither — the builtin answers after all.

`whereis` answers the other question — every place a name lives, plus its manual pages — and prints
one entry per *thing*: the copy of a stored script that `oslo macros` writes for other shells is the
same macro written down twice, not a second place, so it is left out here as it is everywhere else.
Manual pages come from `$MANPATH`; sources are not searched, because the directory list that would
mean is different on every machine.

### Autoloaded functions

`~/.config/oslo/functions/NAME.sh` defines `NAME`, and is not read until something calls it.

```sh
$ cat ~/.config/oslo/functions/gitroot.sh
gitroot() { git rev-parse --show-toplevel; }
```

fish's `functions/` directory, and the reason it is worth copying is arithmetic: a `conf.d` snippet
defining twenty functions costs twenty definitions on **every** shell start, including the hundred
short-lived ones a build spawns. An autoloaded one costs a `stat` on the call that needs it.

**It can never shadow anything.** The file is read only after the `$PATH` search has already
failed, so a file called `ls.sh` is dead — `ls` resolves to the program long before. fish lets an
autoloaded function override a command; a shell that promises scripts see POSIX behaviour cannot
have a file on disk quietly redefining `test`. Autoloading adds names, it never changes them.

Abbreviations expand as you type the space that ends the word — `gco ` becomes `git checkout `,
and the line that runs is the one you can read. The `abbr` builtin defines them too, and by hand at
the prompt it is the shorter thing to type; `oslo.abbr` is for the ones you want every session.

### The prompt

Four render keys, each a function returning a string:

```lua
oslo.prompt.left         = function(p) return p.cwd .. " > " end
oslo.prompt.right        = function(p) return p.duration_ms and (p.duration_ms .. "ms") or "" end
oslo.prompt.continuation = function() return "… " end
oslo.prompt.transient    = function() return "> " end   -- redrawn once the line is accepted
oslo.prompt.title        = function(p)                  -- the terminal tab, fish's fish_title
  return p.command and (p.command .. " — " .. p.cwd) or p.cwd
end
```

Every one of them is handed the same facts: `status`, `duration_ms`, `cwd`, `branch`, `user`,
`host`, `language`, `vimode`, `cols`, `jobs`, `continuation`, and `command` — which is set only
while something is running, so `title` can name it and go back to the directory afterwards. A shell-side integration reads the
same things from `$?`, `$EPOCHREALTIME`, `$PWD` and `$OSLO_MODE`, and can own both columns through
`$PS1` and `$RPS1`.

`oslo.prompt.transient` is the one that has no equivalent elsewhere. zsh users build it by wrapping
ZLE widgets — powerlevel10k and oh-my-posh each spend several hundred lines on it — because zsh has
no way to say "redraw the accepted line differently". oslo owns its own editor, so it is one key.

### The hooks

Twenty moments, named `pre-`, `post-` or `on-`. Kebab-case cannot be a Lua field, so every one is
also spelled with underscores — `oslo.on.pre_cmd` and `oslo.on["pre-cmd"]` are the same hook.

```lua
oslo.on.pre_cmd(function(c)  end)   -- { text, cwd, mode }   may answer
oslo.on.post_cmd(function(c) end)   -- + { status, ok, duration_ms }
oslo.on.pre_change_dir(function(d)  end)   -- { from, to }   may answer
oslo.on.post_change_dir(function(d) end)   -- { from, to }
oslo.on.pre_prompt(function()  end)
oslo.on.post_prompt(function() end)        -- once the prompt is on screen
oslo.on.pre_mode_change(function(m)  end)  -- { kind = "vi"|"language", from, to }
oslo.on.post_mode_change(function(m) end)
oslo.on.on_history_open(function(h)   end) -- { seed }
oslo.on.on_history_select(function(h) end) -- { line }
oslo.on.on_history_close(function(h)  end) -- { chosen }
oslo.on.on_completion_start(function(c)  end) -- { word, line, count }
oslo.on.on_completion_select(function(c) end) -- { value, word }
oslo.on.on_completion_cancel(function(c) end) -- { word }
oslo.on.on_job_finish(function(j) end)     -- { id, pid, text, status }
oslo.on.on_time_report(function(t) end)    -- { real_ms, user_ms, sys_ms } — `time cmd` only
oslo.on.on_command_not_found(function(name) end)
oslo.on.on_idle_timeout(function(i) end)   -- needs oslo.misc.idle_timeout
oslo.on.on_exit(function(e) end)           -- { status }
oslo.on.on_key(function(k) end)            -- every keystroke, before the editor acts
```

**Three may answer; the rest observe.** `pre-cmd` may return a replacement line or `false` to
cancel the command; `pre-change-dir` may return `false` to refuse the move; `on-command-not-found`
may return a status meaning it handled things. Everything else has its return value discarded —
there is nothing coherent for `post-prompt` to veto, and a `pre-mode-change` that could refuse
would let a config make vi mode inescapable.

```lua
oslo.on.pre_cmd(function(c)
  if c.text:match("^rm %-rf /%s*$") then return false end
  return nil
end)
```

`preexec`/`precmd`, `postexec`/`postcmd`, `prompt`, `cd`, `command-not-found` and `key` are the
names oslo shipped first and still answers to; each is an alias of the `pre-`/`post-`/`on-` name
above and fires on the same list, so no config breaks. `cd` is an alias of **`post-change-dir`**,
since it always fired after the move.

`post-cmd`'s `cwd` is where the command *ended*, so the pair reads correctly across a `cd`, and it
fires whether the command succeeded or not. `post-change-dir` fires from the one place every `cd`,
`pushd`, `popd` and jump passes through — so it also catches a move made inside a function.

**A hook that observes may change the shell.** Setting a variable, sourcing a file, defining an
alias: all of it works from any `post-` or `on-` handler, including `post-change-dir`, which is
what a `direnv`-style integration needs.

```lua
oslo.on.post_change_dir(function(d)
  if oslo.fs.exists(d.to .. "/.envrc") then
    oslo.source("/dev/stdin")            -- or whatever the integration needs
  end
end)
```

That has not always been true. `post-change-dir` fires from inside `cd`, which is holding the
shell's state, so a handler that tried to change anything met "shell state is busy" and did
nothing — and the message named `oslo.register_builtin`, which the config had never used. Handlers
that only observe are now held until the shell is idle and run there, so the fire site stays where
it is accurate and the handler runs where it can act.

The three that **answer** still run inline, because a `pre-cmd` veto has to be read while the
command is still stoppable. They are handed everything they need as arguments — `pre-change-dir`
gets `d.to` rather than having to ask the shell where it is going — and an `oslo.*` call from one
that reaches for shell state raises where you can see it.

None of this reaches a script: a config is only read by an interactive shell, so nothing here can
change what `sh -c` or a `#!/bin/sh` file does.

`on.key` sees every keystroke — ordinary characters included — before any binding, before vi, and
before the editor acts. It is told `{ name, char, text, cursor, word, word_start }`, and what it
returns decides what the key does:

```lua
oslo.on.key(function(k)
  if k.name == "ctrl-d" and k.text ~= "" then return false end   -- swallow it
  if k.char == "(" then                                          -- rewrite the line
    return { text = k.text:sub(1, k.cursor) .. "()" .. k.text:sub(k.cursor + 1),
             cursor = k.cursor + 1 }
  end
  -- anything else: the key does what it always did
end)
```

Returning nothing is the safe default, and so is returning something unrecognised — only an
explicit `false` swallows a key and only a string or a `{ text = … }` table replaces the line. A
session with no `key` handler attached does not pay for the hook at all.

### Chains

`a && b || c` is one line and several links, and the shell used to keep only the last status. It
now records each one — including the links it **never ran**, which is neither success nor failure
and which nothing else in a shell writes down.

```
> make clean && make build && make test
oslo: chain stopped — resume with: make build && make test

> chain
   make clean     ok               5ms
&& make build     failed (2)     412ms
&& make test      skipped
                  total          417ms

> chain resume
make build && make test
```

`$PIPESTATUS` answers the same question one level down, for the stages inside a single pipeline,
and still does.

### What gets written down

`oslo.on.pre_record` runs when a line has finished and before it is recorded. It is told what ran,
where, how it went, and its links; it answers with the lines to remember.

```lua
oslo.on.pre_record(function(c)
  -- c.text, c.cwd, c.mode, c.status, c.duration_ms, c.profile
  -- c.segments = { { text, op, status, ran, ms }, … }
  for _, s in ipairs(c.segments) do
    if s.text:match("^cc ") then
      return { c.text, s.text }        -- the chain, and that link as its own command
    end
  end
end)
```

| returned | recorded |
|---|---|
| nothing | the whole line — the default |
| `{ c.text, s.text }` | the chain **and** that link, so typing `cc` suggests `cc -c 'd'` |
| `{ s.text }` | only the link; the chain is not remembered |
| `false` | nothing at all |

A link never becomes a command on its own unless a rule asks: a chain is one thing you meant, and
`aa` inside one is not something you typed.

**A rewritten line says so.** The row carries a flag, so anything reading the history can tell
"this is what was typed" from "this is what a rule kept". That is the difference between a
transformation and a hole, and it is why filtering is a rule rather than a switch — recording
itself cannot be turned off, and `oslo.feature` refuses the name.

### Drawing, and taking over what the shell draws

`oslo.ui.block` is a headline and a rail of labelled rows — the shape every report oslo prints
already has, and the same code it uses to print them.

```lua
local b = oslo.ui.block("direnv loaded")
b:row("PATH", "/nix/store/…:/home/…/target/debug", { overflow = "ellipsis" })
b:row("aliases", "_b _c _r _t _v")
b:done()
```

```
direnv loaded
  │ PATH    /nix/store/…:/home/…/target/de…
  │ aliases _b _c _r _t _v
```

A row that does not fit does one of three things, and which one is yours to choose:

| `overflow` | |
|---|---|
| `count` | cut, then ` +12` — for a list of names, where the count is the information |
| `ellipsis` | cut, then `…` — for one long value, where the front is what matters |
| `wrap` | continue on the next line, rail kept — for text that has to be read |

A misspelt policy is an error, not a silent default. `b:done()` writes the whole block at once, so
it cannot interleave with a command's output, and a block drawn into a pipe has no rail and no
colour. `b:lines()` gives the rows back instead of printing them.

**`on-report` lets a config draw what the shell was going to draw.** Five kinds — `direnv`, `job`,
`slow`, `chain`, `time` — and returning `true` means you handled it, so oslo prints nothing.

```lua
oslo.on.on_report(function(r)
  if r.kind == "job" then
    oslo.ui.block(("[%s] %s"):format(r.id, r.text)):done()
    return true
  end
end)
```

Returning anything else leaves oslo's own rendering alone, so a handler that cares about one kind
costs nothing for the rest.

This is not `on-job-finish`, which stays. That one answers "this happened" and may be deferred to a
moment when the shell is idle; this one answers "how should this look" and must be answered before
the default is drawn. Merging them would mean a handler that merely *logged* a job silently
*suppressed* its notice.

**A handler may always draw; it may not always change the shell.** `direnv` and `slow` fire from
the read loop with nothing locked, so the whole `oslo.*` API works. `chain`, `job` and `time` fire
from inside a builtin or the executor — `oslo.ui.block` is fine there, but `oslo.env.set` raises,
loudly and by name. Every field a handler could want is passed in for that reason.

### Features you can turn off

```lua
oslo.feature.set("direnv", false)     -- now
oslo.feature.get("direnv")            -- false
oslo.feature.list()                   -- { name, on, about }, all of them
```

`direnv`, `vi`, `suggest`, `abbr`, `notify`, `marks`, `finder`, `rm`. A name that is not one of
those is an error rather than a shrug — a config that turns off `direnvv` and is quietly obeyed
looks exactly like a config that is not being read.

Turning off a feature that provides a builtin hands the name back to `$PATH`, everywhere at once:
the dispatcher, `type`, `command -v` and completion all agree. That is the point of the `direnv`
one — oslo's builtin reads `.env.lua` and cannot read an `.envrc`, and the real `direnv` can:

```lua
oslo.feature.when("direnv", function(dir)
  return not oslo.fs.exists(dir .. "/.envrc")
end)
```

**`when` is re-asked on every directory change**, so walking out undoes what walking in did. There
is nothing recorded and nothing to restore, which is where "disable on entry, put it back on exit"
normally goes wrong. It is asked for the directory a shell starts in too.

**A feature is a mask over your configuration, never an assignment to it.** `oslo.vi.enabled` says
what you asked for; the feature says whether it applies right now. So turning a feature back on
restores exactly what the config said — a shell configured for emacs does not acquire vi mode by
having the `vi` feature enabled — and a handler never has to remember a previous value.

`set` on a feature that a `when` predicate owns is refused, because the write would appear to work
and then be undone by the next `cd`.

**History and the frecency ranking are deliberately not features.** They are what the command log is
built from, and something reading it is entitled to assume it is complete rather than "complete
except where a config had an opinion" — a gap nobody can see is worse than no data. `redact` and
`--profile` are the controls that exist for this, and both leave a record that is honestly shaped.
A test refuses the names outright.

### Universal variables

```sh
universal THEME=dark        # here, in every other running oslo, and next session
universal -x EDITOR=hx      # and exported to children
universal -e THEME          # gone, everywhere
universal                   # list them
```

fish's `set -U`, under a name oslo can spell — `set` is POSIX's, and its options are shell options
and positional parameters. Stored in `$XDG_DATA_HOME/oslo/universal`, one variable per line, and
**never sourced**: a file every one of your shells writes to is not a file to execute.

Running shells pick up a change before the next command, not the next login. That costs one `stat`
per prompt, since the file is only read when it has actually moved.

**A local assignment wins.** A universal `PAGER` must not stop `PAGER=cat cmd` meaning `cat`, so
the file fills in names this shell has not set and loses to any it has — including for erasure,
where a value you assigned over is yours to keep.

### `status`

```sh
status is-interactive || return    # the line every dotfile repo opens with
status is-login
status current-function            # or `status function` for the whole stack
status basename
status terminal                    # selected terminal features and their origin
```

The predicates answer through the exit status, so they compose with `&&` and `||`. The portable
spelling of the first line is `case $- in *i*) … esac`, which is correct and which nobody remembers.

### `messages`

```sh
messages                 # everything this session said, oldest first
messages -n 10           # the last ten
messages plugin          # only what the plugin loader said
messages --errors        # only what failed
```

What a session said after it has scrolled away — a plugin that could not load, a config file that
raised, a hook that failed, a prompt segment that is quietly falling back. A config now loads
`conf.d/*.lua`, plugins, prompt segments and timers, and any of them can fail in one line twenty
commands ago.

**In memory, and only this session.** It is not a log: nothing rotates, nothing needs permissioning,
and a hook that echoed a token into a warning does not write it to disk. A repeated line is counted
rather than kept twice, so a prompt segment failing on every draw cannot push the startup failure out
of the buffer.

It is a builtin rather than `oslo messages` because a tool is a new process, which has said nothing —
the same reason `:messages` is a command inside neovim rather than a flag to it. Lua reaches the same
buffer:

```lua
oslo.messages.warn("notes", "the database moved; the old one is still there")
for _, said in ipairs(oslo.messages.all()) do print(said.source, said.text) end
```

`oslo.proc.capture`, `sh.df()`, `sh.ps()`, `sh.ls()`, `sh.stat()`, `oslo.path.*`, `oslo.json`, `oslo.re`,
and a `did you mean` drawn from the command index oslo already keeps.

## Building

```sh
make build        # static musl release, the same binary the release action ships
make dev          # a plain debug build, for iterating
make verify       # fmt, line limits, README paths, tests, clippy, rustdoc — all of it
make install      # to /usr/local/bin
nix build         # static musl binary
```

### Optional features

All six are off *by default*, and off for the same reason: a shell that is going to be `/bin/sh`
should carry what every session needs and nothing else. `make build` turns them on, because
somebody building from source is asking for the shell rather than for the floor; the published
release artifact is the default build.

Each cost is what turning that one feature *off* takes back out of the full build, measured on the
static musl binary — 5,266,368 bytes with none of them, 6,013,312 with all five:

| feature | costs | brings |
|---|---:|---|
| `vista` | +341 KB | the model: `predict` as a suggestion source, `oslo.repair`, `oslo.predict.*`, and the correction drawn after a mistyped line |
| `direnv` | +200 KB | `.env.lua` and `.envrc` read on arrival in a directory, the `direnv` builtin, `oslo.direnv` |
| `nix` | +60 KB | `oslo.nix` — every `nix --json` answer as a Lua table, and flake-output completion |
| `scratch` | +48 KB | named sessions that outlive their terminal, and the key that finds them |
| `plugin` | +108 KB | `oslo plugin` — installing somebody else's Lua, and loading it on first use. `oslo.db` and the `pre-cmd` veto a plugin is written against are in **every** build |
| `argc` | +308 KB | a script declares its options in comments and the shell parses them: the `argc` builtin, `oslo --argc-eval` for bash scripts, and completion from those comments. The largest of the six, and the only one that vendors a parser |

```sh
make build                  # static release, every feature on
make build TYPE=minimal     # static release, none of them
```

**There are no others**, and in particular none that exist to serve the test suite —
`--all-features` turns on exactly the five above. Test-only helpers that other crates' tests need
are ordinary `pub` items the linker drops from the binary, not features, because a build flag
should never decide whether test scaffolding ships.

A config is written to work either way, because a build without the feature simply does not have
the name:

```lua
oslo.keys["f4"] = function(line)
  return oslo.repair and oslo.repair(line.text) or line.text
end
```

`oslo.suggest.sources` still *parses* `"predict"` without the feature — a config is shared between
machines, and a source that cannot answer is skipped exactly like one that had nothing to say.

Every `.rs` file is under 600 lines, enforced by `scripts/check-loc.sh`.

To make it the system `/bin/sh` — the symlink, the dpkg diversion that survives a dash upgrade,
what to check afterwards and how to undo all of it — see
[docs/default-shell.md](docs/default-shell.md).

## Known gaps

`for ((;;))` with touching separators, process substitution without `/dev/fd`, `coproc`, `select`,
associative arrays, and a structured tool reading the shell's own stdin. Each one is listed with its
cause and its workaround in [docs/known-gaps.md](docs/known-gaps.md), most of them pinned by a test
that fails if the gap ever closes.

## Licence

MIT.
