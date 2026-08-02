# What to build next: fish, zsh, Hilbish and nushell, read for parts

Eight agents studied four shells — two each, on different angles — and a ninth ranked what came
back. This is that ranking, kept because the reasoning is worth more than the list: every entry
says where it was found and which oslo file it was checked against, so a claim that has gone stale
can be spotted rather than inherited.

The companion document, `dual-channel-pipe.md`, is the design for the structured pipeline. It is
separate because it is the thing that will be re-read while it is being built.

## What each study concluded

- **fish-interactive** — Abbreviations (fish's `abbr`) are the single highest value-to-cost thing oslo is missing: they give the ergonomics of aliases without touching script semantics, which matters enormously for something meant to be /bin/sh — and they need the same buffer-rewrite substrate (a `commandline`-equivalent plus Lua-callable key bindings) that everything else on this list is blocked on.
- **fish-scripting** — fish's completion engine is not a feature, it is an architecture oslo does not have: per-command completion files autoloaded lazily on first Tab from a search path, written in a declarative spec language (`complete -c ... -s ... -l ... -n cond -a args -r/-f/-F/-k -w`), with a man-page scraper filling the gap for everything nobody wrote a file for. fish ships 1066 such files (git.fish alone is 3046 lines / 1671 `complete` calls); oslo has four hand-written Rust specs (src/interactive/spec/definitions/) plus one imperative Lua hook that lives in init.lua and returns bare name+description pairs. Adopting the autoload + declarative-spec + man-page-fallback trio is the highest-leverage thing available, and it also makes fish's existing corpus mechanically importable.
- **zsh-interactive** — zsh's completion is better because matching is a declarative per-position transform tried in a fallback chain (`matcher-list`: `m:{a-z}={A-Z}`, `r:|[._-]=*` so `/u/s/b` completes `/usr/share/bin`), configured per-command through a context-addressed style table — where oslo has one global `starts_with` test; building the matcher engine plus `zstyle`-style contexts is the highest-leverage interactive work available, and everything else on this list (tags, completers, `_arguments`, menu filtering) hangs off those two.
- **zsh-language** — zsh's two most powerful language features — glob qualifiers (`*(.om[1,5])`) and parameter-expansion flags (`${(f)...}`, `${(s.:.)...}`) — are both implementable entirely inside oslo's own code with zero changes to the vendored brush-parser, because brush already tokenizes `*(.)`, `!(foo)` and `*(#q.)` as ordinary words (verified against target/release/oslo) and oslo re-lexes every `${...}` body itself in src/lexer/param.rs; they are additive to POSIX (both are syntax errors or literal text today), so the highest-value zsh borrowings are also the lowest-risk and lowest-cost ones.
- **hilbish-api** — Hilbish's `sinks` — a Lua-defined command receives `{input, out, err}` stream objects bound to the *real* pipeline fds, not the process's — is exactly the substrate oslo's dual-channel pipe needs, and oslo's `register_builtin` callback signature (`function(argv)` only, writing via `print` to process stdout) should be widened to `function(argv, io)` before it has users, because the structured channel is just a fourth sink on the same object.
- **hilbish-design** — Hilbish's `commander.register(name, fn(args, sinks))` — commands written in the config language that receive real input/output stream objects wired to their pipeline stage — is the missing floor under oslo's dual-channel pipe: oslo's `register_builtin` gets only an argv table and returns only an exit status, so there is no channel to hang the structured output on yet. Build the stream face first (`sinks.input`/`out`/`err` over real fds, never buffers — Hilbish's buffered sinks silently ate output in issues #344/#352), then the structured channel is one more field on the same object.
- **nu-pipeline** — The pipe cannot sniff structure after the fact — nushell's actual mechanism is that the *producer is told where its output is going before it runs* (`OutDest`), computed from the consumer's declared input type. Build that decision (typed command signatures + an OutDest passed down) first; the dual-channel to external programs then falls out as an env-gated side descriptor (`STDDATA_FD`), which is the one design that stays POSIX-safe.
- **nu-commands** — A dual-channel pipe is not the hard part — the hard part is that on day one only oslo's own five tools would have a structure channel, and nushell's actual answer to that problem is a declarative adapter layer (`from json` / `parse "{a} {b}"` / `detect columns`) plus `content_type` metadata that auto-applies it. Build the adapter registry alongside the side channel, or the structured world is five commands wide and nobody uses it.

## The recommendation

Build the dual-channel pipe, but start it at the wrong end on purpose — at the adapters, not at
the tools.

The trap in the owner's framing is that "our tools will output both structured and normal" makes
the feature useless until his tools exist. On day one a structured world would be five commands
wide and he would be the only user. The escape is `from json` / `lines` / `parse` / `where` /
`each`: those work today, with kubectl, docker, gh, systemctl, jq, cargo and lsblk, with zero
oslo-aware programs. That is the demo that makes the feature real on the machine he is running
now, and it is also the implementation strategy for every native tool later, since
`src/lua/api/tools.rs` already commits to parsing external output rather than reimplementing it.

The second correction is mechanical: the pipe cannot sniff. By the time output exists the producer
has already chosen a form and paid to render it. The destination must be decided before the
producer runs, by walking the pipeline right-to-left and reading static `accepts`/`produces`
declarations off the registered tool. Get that inversion wrong and the whole thing is unbuildable.

Concrete three-month shape. Weeks 1-2: ranks 1-6 — six small interactive items, each independently
shippable, each hit every day, none of which touch the pipe. Weeks 3-11: ranks 7-11, the pipe, in
strict order Val model → registry + planner + the POSIX counter test → adapters and `where`/`each`
→ exit verbs → native `df`/`ps`/`ls`. Week 12: rank 12-13 if there is room. Ranks 14-16 are the
next quarter, not this one.

Land rank 8 (registry + `plan()`) with the 375-script differential corpus green AND an
instrumented counter asserting the corpus never once enters the structured path. That converts
"POSIX is safe" from a claim into a build failure, and it is the single commit that makes
everything after it defensible.

Two decisions to make before writing any code, because both are cheap now and cross-cutting later:
(a) widen `oslo.register_builtin` from `function(argv)` to `function(argv, io)` — announce it in
week 1 even if the `io` object is empty, so nobody builds on the narrow signature; (b) make
`Table`'s hash part insertion-ordered (`src/lua/eval/value.rs:150`), which fixes a real
nondeterminism oslo has today and is a hard prerequisite for stable column order.

And one rule, worth writing into the design doc rather than leaving to be rediscovered: fd 1 is
never used for structure, under any option, in any mode. The display rendering and the transport
rendering are two different functions. Nushell's most damaging bug is that piping structure to an
external renders box-drawing characters onto its stdin; if `│ 4.2G │` ever reaches a pipe in oslo
the project's premise is dead.

## The order

Ranked by value to someone using this as their daily shell, divided by effort, with dependencies
respected. Ranks 1-6 are interactive work that touches nothing structural; 7-11 are the pipe, in
strict order; 12 onward is the following quarter.

### 1. Prefix history search on Up, and stop destroying the in-progress line

*1-2 days* · found in zsh-interactive (history-beginning-search-backward, up-line-or-beginning-search); verified against src/startup/keybind.rs:74-122

Up filters history to entries starting with what is already typed, cursor left at the end of the
typed prefix; walking back past the newest entry restores the line the user was composing instead
of blanking it. src/startup/keybind.rs:74 walks the whole per-language history unfiltered, and its
own comment at line 110 records that depth 0 returns an empty line because 'the editor keeps no
copy of it for us'. Keep the copy.

**Why.** The most-pressed key in a shell, currently the weakest version available, plus a genuine data-loss
papercut: pressing Up twice deletes what you were typing. oslo already has sqlite history with a
language column, so prefix filtering is a WHERE clause.

### 2. command-not-found hook

*1-2 days* · found in fish (fish_command_not_found), hilbish (command.not-found); verified at src/exec/simple.rs:277 and src/lua/api/shell.rs:29

A fourth entry in HOOKS (src/lua/api/shell.rs:29, currently exactly ["precmd","postcmd","cd"])
plus one call site at src/exec/simple.rs:277, which today hardcodes report_unrunnable(...,
"command not found", 127). The hook receives the name; if a handler runs and returns a status, use
it. Pair it with a did-you-mean drawn from src/interactive/command_index.rs, which already knows
every command on PATH.

**Why.** The single integration point a distribution's package manager most obviously wants — 'nvim is in
package neovim'. Every other distro bolts this on as a bash function; oslo can have it as a real
hook for the cost of an array element. The typo-correction half is free because the command index
already exists.

### 3. Key bindings that run Lua and can rewrite the buffer

*~1 week* · found in fish (commandline, bind), zsh (ZLE widgets, $BUFFER/$CURSOR), hilbish (editor:insert/deleteByAmount/getLine/log); verified against src/interactive/keys.rs and src/startup/keybind.rs

oslo.keys currently maps a key name to one of seven fixed actions (src/interactive/keys.rs:18-29).
Allow the value to be a Lua function instead, called with a line object: read buffer, read/set
cursor, read the current token and the current pipeline stage, replace/insert/append, ask whether
the line parses (oslo already computes this for PS2 in src/interactive/syntax.rs), and log a line
above the prompt without corrupting the edit. rustyline's ConditionalEventHandler already gives
ctx.line()/ctx.pos() and Cmd::Replace, so the substrate is reachable without replacing the editor.

**Why.** This is the substrate item. Abbreviations, alt-e, sudo-prefix, fzf-style pickers, expand-in-place
and a key that rewrites a pipeline are all one feature — 'let Lua see and change the line' — and
none are buildable without it. It is also oslo's largest API inconsistency: prompts, themes,
completion, suggestions and hooks are programmable and the keyboard is not.

### 4. Abbreviations

*2-3 days* · found in fish (abbr), hilbish (nature/abbr.lua, ~60 lines over the editor API) · depends on rank rank 3

abbr add gco 'git checkout' — typing gco then space rewrites the buffer in place to the literal
text before anything runs. Ship only the 20% that matters: command-position and anywhere
placement, a cursor marker, and a function-valued expansion. Explicitly not fish's
--regex/--command/--rename surface.

**Why.** An alias is a language feature: it changes what a script means, it is invisible afterwards, and
history records 'gco' rather than what ran. For a shell whose pitch is 'replace /bin/sh and do not
change what scripts see', an interactive-only expansion that leaves the real command in the
buffer, in history, and in the sqlite language column is strictly the better primitive. oslo
already highlights and ghost-suggests, so the user watches the real command appear.

### 5. Restore terminal modes around foreground jobs

*1-2 days* · found in hilbish (terminal.saveState/restoreState, added for issue #136); verified by grep across src/

Snapshot termios before handing the tty to a foreground external and restore it when the shell
takes the tty back. Grep finds tcgetattr/tcsetattr only in src/interactive/dropdown/mod.rs,
src/interactive/query.rs and src/env/builtins/io/read_input.rs — nothing around external
execution. src/exec/job/control.rs does the process-group and tcsetpgrp handover correctly; the
terminal *modes* are simply not part of it.

**Why.** A killed TUI, a crashed vim, or a program like cava leaves echo off or the alternate screen
active, and rustyline then captures the broken state as its 'original' and restores it after every
line. 'My terminal is broken until I type reset' is the class of bug that sends people back to
bash in week one, and a shell meant to be /bin/sh will meet it constantly.

### 6. Alias-aware completion and --wraps

*1-2 days* · found in fish (complete --wraps); verified against src/interactive/completion.rs

spec_candidates looks up word.prior_words[0] verbatim (src/interactive/completion.rs), so after
'alias g=git', 'g comm<TAB>' offers nothing — even though the alias table is already loaded and
oslo already renders alias expansions in a second dropdown column. Resolve the head through the
alias table (transitively, with a cycle guard) before spec lookup, and add an explicit wraps
declaration for wrapper scripts and sudo/doas. Also fix src/interactive/completion.rs:76, where a
config-supplied candidate's kind is hardcoded to "option" so the dropdown's kind column lies.

**Why.** Everyone aliases git. A large fraction of a distro's commands are wrappers (podman→docker,
batcat→bat, a local pkg front). This is the cheapest correctness fix on the list — the data is
already in hand at that point in the function.

### 7. The pipeline value model: src/data/

*2-3 weeks* · found in nu-pipeline (Value variants, Record ordering, ByteStream, Signals); Lua Table verified at src/lua/eval/value.rs:147-155 (HashMap hash part) · depends on rank insertion-ordered Lua table

A new Rust enum, NOT lua::eval::value::Value. Val { Null, Bool, Int, Float, Str, Bytes, Size(u64),
Duration(i64), Time(i64), List, Record, Error }. Record is two parallel Vecs with linear lookup —
insertion-ordered, because records are 3-15 columns wide and order decides how columns are drawn
and serialised. A table is List(Record); there is no fourth type. Data { Value, Rows(RowStream),
None } where RowStream carries an interrupt flag and an optional column header hint. Two renderers
written as separate functions from the start: render_display (colour, borders, width-fitting,
human sizes) and render_transport (plain, one record per line, untruncated, un-abbreviated).
Prerequisite: make Table's hash part insertion-ordered (src/lua/eval/value.rs:150).

**Why.** Independently useful before any pipeline change: rewrite
sh.df()/sh.ps()/sh.ls()/sh.stat()/sh.env() (src/lua/api/tools.rs) to build Val and convert at the
Lua boundary. That proves the model against five tools that already exist and already have tests,
and Val::Size replaces the hand-rolled size/size_human pairs the doc itself names as a drift risk.
Val::Error as a variant is the load-bearing part nobody remembers: ps hits a process that exits
mid-scan, df hits a stale NFS mount; text tools warn and continue, and that is why people trust
them. The interrupt flag goes in the constructor now because nushell's decade of uninterruptible
loops is the direct consequence of adding it late.

### 8. Tool registry, the right-to-left planner, and the POSIX safety assertion

*2-3 weeks* · found in nu-pipeline (OutDest, Signature::input_output_types); the supplied design; verified against src/exec/pipeline/mod.rs:327-394 · depends on rank rank 7

A Tool registration carrying name, run, rows, accepts, produces, collects. plan(stages) ->
Vec<Sink> where Sink is Print | Text | Rows, computed right-to-left from static declarations,
never from bytes. An edge is Rows only if the producer declares it, the consumer declares it, no
redirection touches either side, and both are in-process. run_stages
(src/exec/pipeline/mod.rs:327) gets exactly one new early branch: if no stage is Rows, call the
existing body verbatim. No new operator — no |>, both because a second operator forces the user to
know which to type and because 'a |> b' is already valid POSIX.

**Why.** Pure risk reduction, and it is what makes every later item safe. Land it with the 375-script
corpus green and an instrumented counter asserting the corpus never enters the structured path —
that turns the POSIX claim into a build failure when it stops being true. The vocabulary-
disjointness argument is what makes it true: structure only flows between two names oslo invented,
so no script written before oslo existed can reach the new path.

### 9. The bridge into structure, and the two consumers that justify it

*2-3 weeks* · found in nu-commands (from/parse/detect columns), nu-pipeline (where shorthand, each as pressure valve) · depends on rank rank 8

from json, lines, parse '{a} {b}', detect columns as structured producers that accept bytes from
any external. where and each as consumers, taking a Lua expression with the record's columns bound
as locals plus 'row' as the whole record: df | where 'free < 1gb', ps | where 'cpu > 10' | each
'print(row.name)'. Optionally a per-command adapter table in config so the pipe applies from-json
automatically for declared commands.

**Why.** This is where nearly all of the day-one value lives, and it is the answer to the ecosystem
objection that sank nushell's adoption: it works with kubectl, docker, gh, systemctl, ip, lsblk,
cargo and jq on the machine the owner is running today, before a single native tool exists. The
Lua-expression form is also strictly better than nushell's bare-field shorthand, because the
escape hatch (each) is the same language as the filter, so there is no cliff — and inventing a
filter dialect would be the exact thing 'nobody should have to learn a new language' forbids.

### 10. The exit door and the day-one verb set

*~1 week* · found in nu-commands (to converters, complete, filters); select refusal verified at src/parser/brush_adapter/mod.rs:19 · depends on rank rank 9

to json, to text, cols, get, sort-by, first/last, length. Plus the `complete` builtin: run an
external to completion and answer one record of {stdout, stderr, exit_code}. Note `select` is
unavailable as a name — src/parser/brush_adapter/mod.rs:19 refuses it as a bash keyword — so the
column verb must be `cols` or `pick`, and that should be settled before any docs are written
because nushell users reach for `select` by reflex.

**Why.** Without `to json` the structured world is a walled garden the moment someone wants jq or curl, and
`to json` is also the honest default when stdout is not a tty. `complete` is a few dozen lines for
the single most annoying gap in shell scripting — capturing stderr separately from stdout and
status without a temp-file dance every script gets subtly wrong. 'ps | where cpu > 10 | sort-by
cpu | first 5' is the sentence that sells the whole feature.

### 11. Native tools with both faces: df, ps, ls, path

*2-3 weeks* · found in docs/built-in-tools.md (oslo's own design), fish (path builtin); sh.* parsers verified in src/lua/api/tools.rs · depends on rank rank 10

Register df/ps/ls as real shell builtins whose run face is defined in terms of the rows face, so
there is one source of facts and one renderer. docs/built-in-tools.md specifies this shape and
says 'No tool is implemented yet' — correctly, because today sh.df() is a Lua-side parser of
external df output and `df` at the prompt is still the external binary. Add `path`
(basename/dirname/extension/normalize/resolve/filter/is/sort) as the first tool designed
structured-first.

**Why.** The demo the owner actually asked for: `df` prints what df prints, and `df | where 'free < 1gb'`
never renders text at all. `path` separately deletes a fork per call from every boot and package
script — a distro's init scripts are largely basename, dirname, realpath and test — and most of
its logic already exists in src/lua/api/path.rs. The registry must make 'run is defined in terms
of rows' structurally true rather than conventionally true, or there will be two df parsers within
a year.

### 12. oslo.register_tool with a sinks object, and the widened builtin signature

*~1 week* · found in hilbish (commander sinks, fs.pipe); verified at src/lua/engine.rs:71-95 and src/lua/api/mod.rs:292 · depends on rank rank 11

register_builtin(name, fn) today hands the callback only argv and takes back a status
(src/lua/engine.rs:71-95); Lua print writes to process stdout and lands in the pipe only by
accident of the fork. Widen to fn(argv, io) where io.input can be read as lines or as rows,
io.out/io.err are bound to the real descriptors this stage got, io.rows:emit() is the structured
channel, and io.sink says which face is wanted so a tool can skip work it will not be asked for.
Sinks must wrap real fds, never buffers — hilbish's buffered sinks silently ate output.

**Why.** Deliberately last: the Lua API should be a face on a proven Rust shape, not a guess that then
constrains it. The one thing that must not wait is announcing the signature change, so nobody
builds on fn(argv). It also fixes a real hole today — a Lua builtin cannot read its stdin at all,
so 'cat f | mycmd' is unusable.

### 13. Completion matching that is a transform, not a prefix test

*~1 week* · found in zsh-interactive (matcher-list, zshcompwid(1)); verified against matches_prefix in src/interactive/completion.rs

Replace the single global case_sensitive prefix test with a short ordered fallback chain: exact,
then case-insensitive, then partial-word on separators (so /u/s/b completes /usr/share/bin and f-b
matches foo-bar). Four passes, first non-empty wins, so you never get fuzzy noise when an exact
match exists. Explicitly NOT zsh's full match-spec language and NOT zstyle contexts.

**Why.** This is the whole of why zsh completion feels qualitatively better, and the narrow version
captures most of it. Everything users describe as 'zsh just knew' comes from here. Kept below the
pipe because it is a comfort improvement on something that already works, not a missing
capability.

### 14. Per-command completion files, loaded lazily, plus a completion query API

*4-6 weeks* · found in fish (completions.html, $fish_complete_path, complete -C); verified against src/interactive/spec/definitions/ and src/interactive/completion.rs:44

Completions live in /usr/share/oslo/completions/<cmd>.lua and ~/.config/oslo/completions/, found
and sourced the first time that command is completed, never at startup. A declarative entry says
which option it is, whether it takes an argument, whether files are still legal, and a predicate
gating when it applies. Plus `oslo --complete 'partial line'` printing value/description/kind so
completion is testable and reachable from outside.

**Why.** oslo's completion knowledge is four Rust files
(src/interactive/spec/definitions/{git,cargo,npm,docker}.rs) plus one global for_command hook that
must dispatch on the command name itself. That model cannot reach a distribution — every new
package would need an oslo release. Autoloading is also the packaging story: the package that owns
the binary ships the file. The query API matters because completion is currently the one part of
oslo that cannot be held to its own standard of differential testing.

### 15. Make globstar, nullglob and dotglob real, per-pattern

*2-3 weeks* · found in zsh-language (glob qualifiers, recursive globbing); verified at src/env/builtins/shopt.rs:72-82 and src/expand/glob.rs

src/env/builtins/shopt.rs currently declares globstar, nullglob, dotglob, nocaseglob and extglob
as fixed(false) and fails loudly when a script sets them. Implement ** as a walker mode that
crosses / (the walker at src/expand/glob.rs already processes one component at a time and already
stats candidates), and offer nullglob/dotglob/nocaseglob as per-pattern qualifiers rather than
shell-wide switches — rm *(N.) cannot accidentally leave nullglob on for the next command.

**Why.** Users assume ** works and their scripts break silently when it does not. `shopt -s extglob`
failing with exit 1 kills a very common bash script's first line outright rather than degrading.
The per-pattern form is strictly safer than the global flag bash offers, and the loud-failure
design was the right call precisely because it made this gap visible rather than wrong.

### 16. Automatic directory ring and numbered stack navigation

*2-3 days* · found in fish (prevd/nextd/cdh/dirh), zsh (AUTO_PUSHD, cd -N); verified against src/env/builtins/directories/

Record every cd (the cd hook already fires, src/lua/api/shell.rs:29), give it prevd/nextd on alt-
left/alt-right, cd -N to jump N back, and cd -<TAB> completing the stack with numbers — using the
dropdown renderer that already exists. pushd/popd/dirs are already built
(src/env/builtins/directories/stack.rs) and keep their explicit semantics.

**Why.** cd - is a one-deep toggle and useless the moment you are three wrong turns deep. Among the
highest-frequency things a person does all day, and oslo is 80% built for it: the event, the
stack, the widget and the frecency store all exist.

## Worth doing whenever there is an hour

- Fix src/interactive/completion.rs:76 — a config-supplied candidate's kind is hardcoded to
  "option", so the dropdown's kind column actively lies for branches, files and hosts. One line.
- Give oslo.on.postcmd the command text and the elapsed duration. Both are already measured two
  lines earlier in src/startup/repl.rs (~244-255) and both are thrown away; a prompt segment
  cannot show 'took 4.2s' today.
- alt-e: drop the current line into $EDITOR/$VISUAL and take back what was saved. Grep finds no
  mention of EDITOR or VISUAL anywhere in src/, and there is no fc builtin either, so there is
  currently no way at all to get a half-typed pipeline into a real editor.
- A PATH-editing builtin that normalises, refuses duplicates and ignores non-existent directories.
  Every package post-install snippet writes export PATH="$PATH:/opt/x/bin" and gets six copies
  of the same entry; getting it right once in the shell beats getting it wrong in fifty
  packages. hash_forget_all is already wired to PATH assignment, so cache correctness comes
  free.
- oslo -P private mode: a session flag that hides old history and writes none. ignore_space and
  ignore_dups already exist (src/interactive/settings.rs, src/startup/history.rs:88), so this is
  the session-scoped sibling — and it matters more here than elsewhere because oslo's history is
  a durable indexed sqlite database, not a text file.
- Type-to-filter in the completion dropdown. src/interactive/dropdown/mod.rs currently does `break
  None` on any unrecognised byte, which both closes the menu AND swallows the character the user
  typed — so refining a selection by typing loses a keystroke. Route unknown printable bytes
  into a filter instead.
- Answer 'am I inside a command substitution' and 'what is the absolute path of the running oslo'.
  The first is how a Lua prompt helper or hook knows its stdout is being captured and must not
  decorate; the second is how a script re-execs its own interpreter, which $0 cannot tell it.

## Deliberately not doing

A shell that chases four other shells finishes none of them. These were considered and declined;
the reason is recorded so the decision does not have to be made twice.

- Cross-process structured transport (OSLO_DATA_FD, a plugin protocol, msgpack framing, Hello
  handshakes, Ack backpressure) — reserve the environment variable names and the RFC 7464 json-
  seq framing in the design doc, implement nothing. Every day-one structured tool is in the
  binary, so structure never crosses a process; the moment it does you own a wire format, a
  version negotiation and a permanent promise to every package in the distro. Nushell's plugin
  protocol is a large permanent surface that only ever works for programs written for nushell.
  Backpressure is the kernel's job and early-consumer-exit is SIGPIPE, both free.
- A `string` builtin (fish's twenty-plus subcommands) — the fork-elimination argument is real but
  this is weeks of surface area for something Lua already covers inside oslo, and oslo's own
  tools are the main consumer. Revisit only if profiling a real distro boot shows sed/awk/cut
  forks dominating.
- Universal variables (set -U) and a change-notification bus — genuinely nice, and the sqlite
  store would make it easier here than it was for fish. It is not load-bearing for anything, and
  'a theme change repaints every open terminal' is a demo, not a daily need.
- The zstyle context-addressed configuration system — retrofitting a context key onto flat
  settings later is the stated risk, but the whole justification is per-command overrides for a
  tag/completer/menu architecture that this plan is explicitly not building. Do not pay for the
  config system of a completion engine you declined.
- Full _arguments / argparse / zparseopts declarative option grammars — subsumed. The tool
  signature work in rank 8 is the same data; build one declaration that produces the parser, the
  --help and the completion rather than a separate option-parsing builtin now.
- Completions generated by scraping man pages — high value, genuinely large, and the wrong shape
  for a distro that can instead ship a completion file with each package (rank 14). Reconsider
  once autoloading exists and the long tail is measurable.
- Associative arrays (declare -A) — declined long before this research, and the reason survives contact
  with the pipe: records live in Val and shell-mode filters are Lua expressions, so structure
  has a landing site without a second shell value shape that nothing else understands.
- emulate sh / named option dialects / LOCAL_OPTIONS — the correct framing for adding non-POSIX
  language features, but this plan adds none. Vocabulary disjointness (structure flows only
  between names oslo invented) is a cheaper and mechanically checkable version of the same
  guarantee.
- A new pipe operator (|>, ||>) — rejected on two independent grounds: a second operator means the
  user must know which to type, which is exactly the new language the owner ruled out; and 'a |>
  b' is already valid POSIX (a redirection of an empty command), so the operator is itself a
  compatibility hazard. Plain | does the right thing because both ends are declared.
- MULTIOS (date >a >b) — silently changes the meaning of an existing POSIX construct that tens of
  thousands of scripts rely on, and the globbing-of-redirection-targets half turns ': > *' into
  a directory-wipe. zsh itself disables it under sh emulation. The gain over 'tee' is
  negligible.
- explore-style TUI table pager, a built-in pager, and an in-shell doc browser — all three are
  real features and none is needed to make structure useful. A TUI is the terminus of the
  pipeline work, not a prerequisite, and it will absorb a month.
- Dataframes, columnar storage, par-each — nushell spends 193 of 564 commands on this; a shell
  that replaces /bin/sh does not need Arrow. Note also that oslo's rows are row-oriented, so do
  not accidentally design for columns.
- A runner registry / pluggable prompt interpreters, and Yarn-style second Lua states —
  speculative generality. oslo has two languages wired as an enum and no third language is
  coming; the interpreter is thread-local by construction, and the async need is already met by
  external-command prompts with a timeout.
- Timers, a notification centre with unread state, and a compiled-config cache — each is small and
  each is a solution to a problem oslo does not measurably have yet. Measure config parse time
  before caching it.
- zmv, always blocks, anonymous functions, =(cmd) process substitution, ${(f)} parameter-expansion
  flags, ${v:t} modifiers — a good list of small language wins, uniformly below the cut on
  frequency-of-use. ${(f)} and :t/:h are the two most defensible; revisit them together, once,
  rather than one at a time.
- ALREADY BUILT, contrary to the studies — do not schedule: @name named directories with tilde-
  safe resolution (src/expand/sugar.rs, oslo.dirs), leading-space history suppression plus
  ignore_dups (src/startup/history.rs:88, src/interactive/settings.rs), mapfile/readarray,
  declare/typeset, =command expansion, and correct job control with setpgid/tcsetpgrp/SIGTTOU
  (src/exec/job/control.rs, which is better than hilbish's).
- ALREADY DECIDED, and the decisions hold — the fixed hook list (a hook name that never fires is
  indistinguishable from a typo), shopt failing loudly rather than lying about globstar/extglob,
  and refusing `select` by name. Rank 2 adds one hook to the fixed list rather than opening it;
  rank 15 implements the globs rather than softening the failure.

