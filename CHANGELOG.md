# Changelog

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

