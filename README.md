# oslo

A POSIX shell in Rust that also speaks Lua, with a structured pipeline that scripts written before
it existed provably cannot reach. Linux only.

```sh
oslo                       # a prompt
oslo script.sh arg1        # run a shell script
oslo build.lua             # a Lua script — same command, no flag
oslo -c 'echo hello'       # run a command
```

---

## Two languages, one prompt

Shift+Tab switches between shell and Lua **in place** — your line, your cursor and your history stay
where they are.

```
bresilla@tron | I | sh  ❯ ls -la                    ❮  (develop)  ~/src/oslo
bresilla@tron | I | lua ❯ for _, f in ipairs(sh.ls(".")) do print(f.name) end
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

Verbs: `where` `each` `cols` `get` `sort-by` `first` `last` `length` `to` `from` `lines` `parse`.
Producers: `df` `ps` `ls`, and anything you register yourself.

### Your POSIX scripts cannot reach any of it

Structure flows only between two stages that **both** declare they understand it, and every name
that can carry a declaration is one oslo invented. A script written before oslo existed cannot name
one, so every edge in it plans to bytes.

That is not a promise, it is a build failure: `tests/posix_stays_on_the_byte_path.rs` runs all 408
corpus scripts and requires zero structured edges. There is no new pipe operator either — `a |> b`
is already valid POSIX, so the operator would itself be the hazard. Design:
`docs/research/dual-channel-pipe.md`.

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
- **A right prompt**, drawn without the save/restore multiplexers fight over
- **Syntax highlighting** that marks a command that does not exist

### Abbreviations

```sh
abbr gco 'git checkout'      # typing `gco ` leaves `git checkout ` in the buffer
abbr gc 'git commit -m "%"'  # `%` says where the cursor lands
```

Better than an alias for a shell that promises not to change what scripts see: the real command
lands in the buffer, in history and in the log, and you watch it happen.

### Keys that run your code

```lua
oslo.keys["ctrl-s"] = function(line) return "sudo " .. line.text end
oslo.keys["alt-u"]  = function(line)
  return { text = line.text .. " [" .. line.word .. "]", cursor = 0 }
end
```

Data in, data out — the handler is told about the line and answers with what it should become.

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

### What it remembers

A second database, beside the history one, recording where you go and what you run there:

| | |
|---|---|
| `dir` | path, visits, last visit, dwell time, git worktree root |
| `run` | the line, the directory, exit status, how many times, total and worst duration |

Which makes the ghost suggestion answer differently depending on where you are standing:

```sh
~/work/alpha ❯ cargo run --ex⏎     # → cargo run --example xyz
~/work/beta  ❯ cargo run --ex⏎     # → cargo run --example abc
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

Every one of the 54 roles — 22 syntax, 19 dropdown, 7 prompt, 5 widget — is settable, and each
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

## The terminal knows what is happening

`OSC 133` semantic marks (so a multiplexer can fold command output), `OSC 7` working directory,
`OSC 0` title, `OSC 8` hyperlinks, `OSC 52` clipboard over SSH, and desktop notifications for
commands that outlive a threshold. A `copy` builtin puts text on the clipboard through the terminal,
so it works over SSH with no X or Wayland helper installed.

## POSIX, where it counts

The language is the real thing: pipelines, redirections including heredocs and here-strings, all the
control flow, functions, `${var:-d}` and the rest of parameter expansion, arithmetic, globbing,
field splitting, job control with proper process groups and `tcsetpgrp`.

Correctness is measured rather than asserted. 408 scripts in `tests/corpus` run under both oslo and
bash and are compared byte for byte, with known differences listed in
`tests/differential/expected_fail.rs` as a two-way ratchet — the suite fails if a listed case starts
passing, so a stale entry cannot survive. 999 unit tests alongside.

## Directory environments

A `.env.lua` in a project, found by walking up from where you are, nearest ancestor wins. **Leaving
puts everything back** — including variables the file unset, and variables that had no value before
and end up with none again rather than with an empty one.

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

**Lua, and only Lua.** `.envrc` and `.env` were both supported for a while and both are gone:
`.envrc` meant either shipping direnv's 1,400-line stdlib or failing on every real file that says
`use flake`, and `.env` is a second grammar for what one Lua line already says. There is no bash
subprocess and no serialised diff in your environment — oslo *is* the shell.

Because it is Lua, a directory can set more than variables — an alias for its test command, say —
and **aliases are restored on the way out too**, so a project's `t` cannot follow you into the next
one and run the wrong tests. Whatever the file prints is grouped under it, repeats collapsed with a
count.

## One shell, several histories

```sh
oslo                            # ~/.local/share/oslo/default.kv
OSLO_PROFILE=claude oslo        # claude.kv instead — the default untouched
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

There used to be two files, `history.db` and `track.kv`. There is one now. Nothing migrates the old
pair — delete them.

## Tools

`oslo --help` lists them. `config`, `profile`, `history`, `direnv` and `hook`:

```sh
oslo history
oslo config
```

**A script always wins.** The operand slot belongs to scripts — POSIX defines the shell as
`sh [options] [command_file [argument...]]` — so a word is only read as a tool when all three hold:

- it contains no `/`
- **no file of that name exists**
- it is one of the five

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

oslo.vi.enabled         = true        -- vi mode; false for emacs only
oslo.vi.cursor_insert   = "line"      -- block / line / underscore, each + " blink"
oslo.vi.cursor_normal   = "block"
oslo.vi.cursor_replace  = "underscore"
oslo.completion.fuzzy   = "smart"     -- off / tight / smart / loose
oslo.completion.max_rows = 15

oslo.finder.enabled        = true     -- the full-screen history search
oslo.finder.key            = "up"
oslo.finder.limit          = 5000     -- distinct commands loaded when it opens
oslo.finder.confirm_delete = true     -- Delete asks before forgetting a command

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
```

### `rm`

`rm` is a builtin, and at a prompt it is a friendlier one: it removes a directory without needing
`-r`, and with `to_tmp` on it *moves* what you delete instead of unlinking it.

```
~/p ❯ rm -v build notes.txt
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
oslo.prompt.left         = function(p) return p.cwd .. " ❯ " end
oslo.prompt.right        = function(p) return p.duration_ms and (p.duration_ms .. "ms") or "" end
oslo.prompt.continuation = function() return "… " end
oslo.prompt.transient    = function() return "❯ " end   -- redrawn once the line is accepted
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
```

The predicates answer through the exit status, so they compose with `&&` and `||`. The portable
spelling of the first line is `case $- in *i*) … esac`, which is correct and which nobody remembers.

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
