# Changelog

## Unreleased

### <!-- 2 -->🚜 Refactor

- Move tracking storage from jammdb to Tagdata

## [0.2.24] - 2026-08-09

### <!-- 1 -->🐛 Bug Fixes

- Pass use flake arguments through to nix

## [0.2.23] - 2026-08-09

### <!-- 1 -->🐛 Bug Fixes

- Never swap in a prompt of another width
- Add the source file a global ignore swallowed

## [0.2.22] - 2026-08-09

### <!-- 1 -->🐛 Bug Fixes

- Two correctness bugs in for and compare
- Stale interrupt no longer eats a command

### <!-- 2 -->🚜 Refactor

- The top of the stack becomes oslo-runtime
- The shell becomes oslo-shell
- The interface layer becomes oslo-ui
- Ask the shell through a trait, not a store
- The bottom of the stack becomes oslo-base
- The interpreter becomes oslo-lua
- Reach a hook without knowing Lua exists

### <!-- 3 -->📚 Documentation

- Note what the dependency glob does not match

### <!-- 4 -->⚡ Performance

- An async prompt lands without a keystroke
- Stop rendering a spawned prompt 4 times
- Find a prefix by search, not by scan
- Stop rebuilding the world on every keystroke
- Drop five copies from the command path
- A plain word skips the lexer
- Publish LINENO and PIPESTATUS only on change
- A word with no wildcard skips the walk
- Share a function body instead of copying
- Share a function body across closures
- A call statement no longer clones its AST
- Score only the names that match
- Honour the redraw flag a key returns
- Skip the scan when nothing is aliased
- Stop asking the kernel twice per command
- An async prompt waits, briefly, for a fresh one
- Fat LTO now that the shell is six crates

### <!-- 6 -->🧪 Testing

- Measure what one keystroke costs

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Keep other people's code in vendor/

## [0.2.21] - 2026-08-08

### <!-- 0 -->⛰️  Features

- Time the phases of a prompt
- Read .envrc with direnv's stdlib
- A directory navigator widget

### <!-- 1 -->🐛 Bug Fixes

- Size the block to what it shows
- The terminal names the shifted key

### <!-- 4 -->⚡ Performance

- Make the async prompt cache usable
- Warm the command table before forking
- Share and cache the nix dev shell

### <!-- 6 -->🧪 Testing

- Widen the margin on the settle window
- Hold the colour depth while asserting
- Stop a test writing the settings global
- Restore the deferred ratchet rows
- A child re-runs the file it inherits

## [0.2.20] - 2026-08-07

### <!-- 0 -->⛰️  Features

- Strange things
- Strange things

### <!-- 1 -->🐛 Bug Fixes

- A reply ends the wait, not the listening
- Ask nothing through a multiplexer
- Rank by the kind of match, then by recency

### <!-- 4 -->⚡ Performance

- Size-optimise dependencies, except the parser

### Build

- The lua derive lives inside full_moon
- Drop thiserror from both parsers and oslo
- Full_moon_derive on syn 2, without indexmap
- Vendor both parsers and strip what they dragged in
- Full_moon without its serde default

## [0.2.19] - 2026-08-07

### <!-- 0 -->⛰️  Features

- The scanner, the badge and the meta columns
- A look for lists, and history is one of them
- Legend, border, fullscreen and placement per widget
- On-report covers all five of the shell's blocks
- Oslo.ui.block, and on-report for direnv

### <!-- 1 -->🐛 Bug Fixes

- Recency orders the list; the caret keeps its surface
- Carry the undo record to child shells
- Reconcile the directory environment at the prompt
- The caret follows the cursor setting, and one atomic frame
- Tabs, the full-width box and the legend rule
- The box, the rule and the spacing around a widget
- Three oslo.ui names that shadowed or dropped
- Oslo.ui.style was installed twice and painted neither

### <!-- 2 -->🚜 Refactor

- Src/interactive is src/ui
- The history screen is a ui look

### <!-- 3 -->📚 Documentation

- Oslo.ui.block and on-report

## [0.2.18] - 2026-08-06

### <!-- 0 -->⛰️  Features

- Pipeline stages, and redaction for a replay
- One read gives a replay everything it needs
- Pre-record decides what is written down
- The log records who typed it and what it did
- Record what each link of a chain did
- Turn parts of the shell off at runtime

### <!-- 1 -->🐛 Bug Fixes

- The last chain survives the command asking about it
- Forget takes a line out of the log too

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Cleanup

## [0.2.17] - 2026-08-06

### <!-- 0 -->⛰️  Features

- \cmd and \\cmd escape the builtin
- Sh.cmd expands globs, oslo.run opts in
- A resize redraws the line

### <!-- 1 -->🐛 Bug Fixes

- A hook that observes may change the shell

## [0.2.16] - 2026-08-06

### <!-- 0 -->⛰️  Features

- A margin at the top edge to match the bottom
- Every context field an external prompt can name
- Add the maki client behind an off-by-default feature
- Twenty named events across the shell
- A builtin rm with a trash at the prompt

### <!-- 1 -->🐛 Bug Fixes

- The cursor is not shown at column one mid-switch
- PWD is set and exported at startup
- The mode is published before the prompt is rebuilt
- Rebuild the prompt when its inputs change
- Export COLUMNS and LINES for child programs
- A burst is not read past the key it needs

### <!-- 6 -->🧪 Testing

- Every documented setting must be assignable

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Cleanup

## [0.2.14] - 2026-08-05

### <!-- 0 -->⛰️  Features

- A lua hook can see every keypress
- Preexec and postexec get the command
- A lua binding can submit the line
- Complete inside a brace list

### <!-- 1 -->🐛 Bug Fixes

- -c takes the first non-option argument

## [0.2.13] - 2026-08-05

### <!-- 1 -->🐛 Bug Fixes

- -o name and the + option forms

### <!-- 3 -->📚 Documentation

- How to make oslo the system /bin/sh

## [0.2.12] - 2026-08-05

### <!-- 1 -->🐛 Bug Fixes

- The finished line drops its ghost
- Sh -c -- cmd runs cmd
- OSLO_ALLHIST=0 means off

## [0.2.11] - 2026-08-05

### <!-- 1 -->🐛 Bug Fixes

- Track src/track/log/tests.rs, drop --lua from the smoke test

## [0.2.10] - 2026-08-05

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Sync Cargo.lock to 0.2.9

## [0.2.9] - 2026-08-05

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Sync Cargo.lock to 0.2.8

## [0.2.8] - 2026-08-05

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Cleanup

## [0.2.7] - 2026-08-05

### <!-- 0 -->⛰️  Features

- A login shell reads /etc/profile and ~/.profile
- Oslo.source runs a shell file in this shell
- OSLO_ALLHIST records sh -c commands
- Run a script a command at a time

### <!-- 1 -->🐛 Bug Fixes

- One warning per file, not per name
- IFS is a set variable; exit keeps the trap status
- Errexit exemption survives a compound
- Comments inside a heredoc substitution

### <!-- 6 -->🧪 Testing

- Assert the command is in the store, not the file
- Boot arch linux with oslo as /bin/sh
- Pin the errexit and-or compound bug

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Rename benches to bench
- Cleanup

## [0.2.6] - 2026-08-04

### <!-- 0 -->⛰️  Features

- Warning box goes last, sized to the help
- Warn about install problems in --help
- Drop --lua, --sh, --no-vi; hide --posix
- Sh implies posix, drop --profile
- Oslo <tool> when no script of that name exists
- Tools by argv0, coloured help, --details
- Drop --vi, vi is the default
- Strict profile names, quicker scanner
- OSLO_PROFILE names a profile too
- Profile in the bar, tab switches it
- Name both stores after a profile, add --profile
- Five scopes on the arrow keys, tracked in the store

### <!-- 1 -->🐛 Bug Fixes

- Right accepts only outside vi normal mode
- Right accepts the ghost suggestion

### <!-- 2 -->🚜 Refactor

- Toggle key is config, not an env var
- Drop the argv0 symlink dispatch
- One store, not two
- Drop the legacy store adoption

### <!-- 3 -->📚 Documentation

- Add an ENVIRONMENT section to --help
- Drop -- from the option list
- Posix mode changes four things, not two
- Two known gaps are no longer gaps

## [0.2.5] - 2026-08-04

### <!-- 0 -->⛰️  Features

- Wider scanner, chevrons in the query colour
- Centre the delete question, bracket the buttons
- Confirm before deleting from history
- Scanner then chevrons in the search bar
- Knight rider scanner in the search bar
- Seed from the line, cursor, marks, delete
- Remove rustyline, oslo owns its line editor
- Finder, abbreviations and lua keys on the native editor
- Vi mode on the native editor
- Multi-line continuation on the native editor
- Completion and ghost hints on the native editor
- Run the shell on the native editor, opt-in
- Example that drives the native editor
- Redraw escapes and the editing state machine
- Ctrl/alt key decoding and the emacs keymap
- Native line buffer and layout engine

### <!-- 1 -->🐛 Bug Fixes

- Atomic redraws and the scanner on its surface
- Drop bold from match marks

### <!-- 6 -->🧪 Testing

- Answer the finder's delete confirmation

## [0.2.4] - 2026-08-04

### <!-- 0 -->⛰️  Features

- Brighter syntax colours, ansi slots untouched
- Universal variables shared between shells
- Autoload functions from functions/NAME.sh
- Title key, status builtin, named frames
- Preexec/prompt hooks, transient prompt, fish settings
- Real facts for every prompt, honest set -o
- Dynamic variables and the config fixes

### <!-- 4 -->⚡ Performance

- Drop duplicated digest stack via sha2 0.10
- Trim regex features, smaller and faster

### <!-- 6 -->🧪 Testing

- Stop a stdin-reading test hanging the suite

## [0.2.3] - 2026-08-04

### <!-- 0 -->⛰️  Features

- The remaining eight gum widgets
- Gum-style input widgets for shell and lua

### <!-- 1 -->🐛 Bug Fixes

- Draw the caret instead of moving the cursor
- An abort is not an ordinary quit
- Ctrl-c cancels and erases the widget
- Run a script the kernel refuses as ENOEXEC
- Stop the widgets eating the transcript

### <!-- 3 -->📚 Documentation

- A tour of the thirteen ui widgets

### <!-- 5 -->🎨 Styling

- A measured rule above every key legend

### Build

- Static musl by default, and accept --login

## [0.2.2] - 2026-08-03

### <!-- 0 -->⛰️  Features

- Full-screen fuzzy history search on up
- Let shell code set the right prompt
- Make bash shell integrations work end to end

### <!-- 1 -->🐛 Bug Fixes

- Swallow escape sequences, restyle the list

### <!-- 2 -->🚜 Refactor

- Remove bind and the command renderer

### <!-- 5 -->🎨 Styling

- Plain rows and a codex-shaped input surface

## [0.2.1] - 2026-08-03

### <!-- 0 -->⛰️  Features

- Run PROMPT_COMMAND before every prompt
- Let shell code claim a keystroke
- Run the DEBUG trap before each command

### <!-- 1 -->🐛 Bug Fixes

- A quoted empty default is still one field

## [0.2.0] - 2026-08-03

### <!-- 0 -->⛰️  Features

- Report what changed, not how many
- A real terminal library, and the escapes to use it
- The api is require-able, and you can write your own
- Nix_develop, the use flake equivalent
- A directory may set the prompt, and give it back
- Restore locals and export flags, add oslo.path_add
- One Lua config file, and direnv helpers from it
- Group and colour what a directory environment reports
- Load directory environments on cd
- The allow gate, env diff and dotenv reader
- The allow gate, the env diff and dotenv
- Fuzzy inline suggestions on by default
- Gap-capped fuzzy matching, off by default inline

### <!-- 1 -->🐛 Bug Fixes

- Signal a job by its spec, not only by pid
- A dev shell must not take your commands away
- The ghost is a continuation again; aliases unload
- Allow and deny take effect where you stand
- A fuzzy match must reach the first character

### <!-- 2 -->🚜 Refactor

- Drop the migrate module
- Group the api into libraries
- Oslo.direnv is a library, not loose functions
- Drop what nothing calls
- One file type, .env.lua and nothing else

### <!-- 3 -->📚 Documentation

- Re-measure size against other shells
- How big oslo is next to seven other shells
- Why --json loses what the bash form keeps
- Oslo's own .env.lua
- Move the known gaps out of the README
- The directory environment design
- The tracking store, the smarter cd and fuzzy matching

### <!-- 4 -->⚡ Performance

- Replace turso with jammdb behind one seam
- Turn off turso features oslo never used
- Tune the release profile, measure the rest
- Cache compiled regexes, fold fuzzy patterns once

### <!-- 6 -->🧪 Testing

- Pin the PATH round-trip

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Drop --locked so a version bump cannot redden every job

## [0.1.4] - 2026-08-02

### <!-- 0 -->⛰️  Features

- *(complete)* gap-capped fuzzy matching, off by default inline
- *(suggest)* fuzzy inline suggestions on by default

### <!-- 2 -->🚜 Refactor

- *(ci)* drop --locked so a version bump cannot redden every job

## [0.1.3] - 2026-08-02

### <!-- 0 -->⛰️  Features

- Pink builtins and a padded sudo field
- Prune the store and seed the ring
- Record every command and suggest by directory
- Jump to a remembered directory
- The store behind a smarter cd
- Frecency scoring and match tiers

### <!-- 1 -->🐛 Bug Fixes

- Record what you run at home, never jump to it
- Checkpoint the log, group heads by tool

### <!-- 3 -->📚 Documentation

- The smart cd design

### <!-- 6 -->🧪 Testing

- Observe a job that is still running
- Do not inherit XDG_CONFIG_HOME

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Install musl-tools for the C deps
- Reuse the cached cargo-fuzz binary
- Check the lockfile at msrv
- Sync Cargo.lock with the 0.1.2 bump

## [0.1.2] - 2026-08-02

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Fix the release smoke test, publish build artifacts

## [0.1.1] - 2026-08-02

### <!-- 0 -->⛰️  Features

- Globstar as a real shopt option
- Match by pieces, not just prefix
- Register_tool for structured commands
- Directory ring with prevd, nextd and cd -N
- Ps and ls as row producers
- To json and the day-one verbs
- Lines, parse and from json
- Df and where, the first structured pipeline
- The planner and the posix byte-path assertion
- The structured pipeline value model
- Abbreviations that expand in the buffer
- Bind a key to a lua function
- Follow aliases, stop claiming a kind
- Command-not-found hook and did-you-mean
- Prefix search on up, keep the composed line
- Named styles and live mode in lua prompts
- Full PS1 escapes and external prompt commands
- Segment-based lua prompts
- One prompt for both languages, with the vi mode
- =command and @name at the prompt
- Mark where the prompt ends and input begins
- Ask the terminal its background and suit the palette
- Notify when a slow command finishes
- Paths in diagnostics are clickable
- Copy builtin puts text on the clipboard over OSC 52
- Report the working directory and set the title
- Vi mode is on by default
- First-class vi mode with per-mode cursor shapes
- Pin syntax colours to rgb and light sudo as danger
- Light globs, numbers, assignments and vars in strings
- A command says which directory it came from
- Host and user left, status and directory right
- Draw a right prompt by default
- Sh.ps() and sh.stat() finish the tool set
- Sh.env() and sh.ls() answer rows
- Sh.df() answers rows instead of text
- HISTSIZE bounds the database and history -c clears it
- Keep history in a database with its language
- Oslo.completion.for_command completes one command
- Oslo.completion.sources filters the kinds offered
- Oslo.completion.sort chooses the order
- Oslo.suggest.accept binds the suggestion keys
- Per-kind info columns, scriptable from Lua
- Refine completion candidate descriptions
- Bind keys from oslo.keys
- Make .oslorc Lua and add the settings surface
- Lua prompts, including a right prompt
- Suggest paths as well as history
- Highlight to fish's depth
- Theme the dropdown and badge the kind
- Emit OSC 133 command boundaries with block ids
- Add oslo.http with curl's certificate rules
- Stream command output with oslo.lines
- Add proc, job and the output converters
- Add introspection, options and hooks
- Read shell or Lua at the prompt
- Share one namespace with shell variables
- Add require, oslo.re and oslo.json
- Add the fs, path and os namespaces
- Add the argv call model and sh sugar
- Run the oslo.* API on oslo's own Lua
- Evaluate Lua without a C interpreter
- Implement process substitution
- Add LINENO, finish round A
- Build printf in, matching bash
- Argv, capture, cd, env, glob, exit
- Detect Lua vs shell, drop --lua-script
- Line editing, rc files, test debt
- Arrays, arithmetic cmds, regex, job control
- Conformance, shell options, traps
- Quoting provenance, params, arithmetic
- Add bash oracle, CI matrix, install

### <!-- 1 -->🐛 Bug Fixes

- Restore terminal modes after a foreground job
- Iterate tables in insertion order
- Never ghost a multi-line history entry
- Seven defects from the audit
- Repaint a wrapped line correctly
- Never write the final column
- Recall follows the language
- Completion and syntax follow the language
- Suggestions follow the live language
- One source of truth for the language
- Redraw the prompt in the current language
- Switch language on the first press
- Do not suggest shell commands in lua
- Keep shell and lua history separate
- Move the line when the prompt changes width
- Hold one width across languages
- Toggle language in place, quiet ctrl-c
- Esc enters normal mode on the first press
- Follow symlinks when indexing PATH
- Mode indicator now matches the editor
- Tab completes in normal mode too
- Repaint only the prompt, never the line
- Repaint the row when the vi mode changes
- Move the vi indicator where it can redraw
- Move back rather than restore the cursor
- Draw the right prompt without save/restore
- Case_sensitive actually decides the match
- A case pattern does not close a substitution
- Export NAME marks it without creating it
- A quoted ! is a class member, not a negation
- Stop the nesting guard refusing real scripts
- Erase rows a shorter page leaves behind
- Tab after accepting no longer undoes it
- Stop the menu eating the prompt; drop the border
- $? sees a command substitution in its word
- Run the shell on a stack oslo reserves
- A hash inside a word is not a comment
- Stop waiting for process substitutions
- Report write errors from echo and printf
- Skip comments when copying a construct
- Only empty parens make a function definition
- Stop substituting aliases twice
- Stop ignoring SIGPIPE
- Substitute aliases before parsing
- Let -- ends the options
- Make set -m turn job control on
- Let break end a loop from its condition
- Stop [[ ]] splitting and globbing operands
- Stop ${x+"$@"} joining its arguments
- Honour quoting inside shell patterns
- Report reserved words from command -v
- Stop nesting guard rejecting configure
- Reap reparented orphans when pid 1
- Stop nesting guard rejecting real scripts
- List inherited traps in subshells
- Honour -n and -a, run subshell EXIT traps
- Drop Linux-only probes and paths
- Use SIGUSR1's number, not Linux's 10
- Bound nesting to fit the smallest stack
- Read limits via getrlimit, not /proc
- Parse PROJECT outside make for 3.81
- Portable PROJECT parsing, richer CI errors
- Close remaining gaps, vendor parser patch
- Correct exit status, fds, subshell state
- Stop crashes, hangs, data-as-code

### <!-- 2 -->🚜 Refactor

- Rename rush to oslo

### <!-- 3 -->📚 Documentation

- Add cliff for changelogs
- Rewrite the readme around what oslo does
- Worked example for the dual-channel pipe
- Shell research and the dual-channel pipe design
- Decide the shape of a built-in tool
- Collapse four plans into one open-work list
- Close out the round C findings
- Locate the isset -x cause in export_var
- Record the nesting-guard and glob fixes
- Record the config and interactive plan
- Record the Lua layer decisions
- Record the builtin.t cause and the suite running
- Fix an unresolvable intra-doc link
- Plan the conformance oracle and pty tests
- Record $(case) gap and its workaround
- Fix readme heading spacing
- Record process substitution results
- Record round C sweep findings
- Note the bash-version gate in the corpus

### <!-- 4 -->⚡ Performance

- Stop blocking on the background query
- Warm the command index at startup

### <!-- 5 -->🎨 Styling

- Ghost text takes colour 240
- Ghost text takes colour 238
- Keep scope.rs inside the line limit
- Recolour the selected row and its badge
- Use colour 241 for the dir badge

### <!-- 6 -->🧪 Testing

- Cover the argv call model in the corpus
- Pin the brush comment bug in the ratchet
- Assert ^Z now the shell is not PID 1
- Boot a real Alpine userland under OpenRC
- Run modernish and job control in the VM
- Cover the four modernish findings
- Boot oslo as alpine pid 1 and /bin/sh
- Add lua corpus with recorded oracles
- Gate corpus cases on oracle bash version

### <!-- 7 -->⚙️ Miscellaneous Tasks

- Drop PROJECT, read metadata from Cargo.toml
- Target Linux only, drop platform gates
- Surface fmt and test failures as annotations
- Split verify into named stages for diagnosis
- Initial commit

### <!-- 9 -->◀️ Revert

- Drop oslo.http in favour of sh.curl

### Build

- Give the dev shell the musl target
- Add tokio and turso for the history database
- Track the brush integration branch
- Track the brush fork until #1253 lands
