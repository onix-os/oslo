# Making oslo fit to be a distro's `/bin/sh`

`PLAN.md` got oslo to "matches bash on 390 corpus scripts". This plan is about a different and
harder bar: **being the `/bin/sh` a whole operating system is built and booted with**, and being a
Lua shell worth writing that system's tooling in.

Everything below was measured against bash on 2026-07-29, not assumed. Where a row says a feature
does nothing, that was reproduced at a prompt.

## What already holds up

Worth stating, because it decides what *not* to spend effort on:

* **~60 distro idioms probed against bash — 55 identical.** `set -e` in all eight positions that
  trip shells up (`if` conditions, `&&` chains, subshells, functions, `!`, `while`, command
  substitution), heredocs including `<<-` and in-function, `getopts` with `OPTARG`/`OPTIND`,
  `${v%%}`/`${v##}`/`${v:=}`/`${v:?}`, `IFS` splitting and re-splitting, `case` globs, `trap` on
  EXIT at top level, `command -v`, `exec` replacing the shell, `wait`, subshell isolation.
* **Performance is not a problem.** A fork-heavy loop (300 command substitutions) runs in 0.12s
  against bash's 0.13s. Pure evaluation is ~3× slower than bash (0.14s vs 0.05s for 20 000
  arithmetic iterations), which is worth knowing and is not what `configure` scripts spend their
  time on.
* **Static musl binaries**, no libc dependency, `readelf`-asserted in CI.
* Deep recursion is bounded, a 5 000-argument list works, `exec` and `wait` behave.

## Round A — things that are wrong now

Ordered by what breaks first when a distro is built on this.

| # | Defect | Measured | Why it matters here |
|---|---|---|---|
| A1 | `set -n` / `-n` **executes the script** | `oslo -n -c 'echo X'` prints `X`; bash and dash print nothing | `sh -n` is how packaging validates maintainer scripts. Running what was meant to be parsed is a security bug, not a missing feature. `noexec` is listed by `set -o`, so it also *claims* to work. |
| A2 | `set -a` (allexport) does nothing | `set -a; V=1; env \| grep ^V=` finds nothing | `set -a; . /etc/os-release` is the most common env-loading idiom there is. It silently exports nothing. |
| A3 | `trap ... EXIT` never runs in a subshell | `(trap "echo sub" EXIT; echo in)` prints only `in` | Subshell cleanup handlers are how scripts remove temp files. Silent non-execution leaves litter, or worse. |
| A4 | `$(trap)` in a subshell reports nothing | `trap "echo hi" INT; saved=$(trap)` leaves `saved` empty; bash gives `trap -- 'echo hi' SIGINT` | The save/restore-traps idiom. POSIX special-cases command substitution here *because* a plain subshell resets traps — so the strings must stay listable even though the actions do not run. `trap` on its own already works; this is only the subshell case. Related: oslo prints `INT` where bash prints `SIGINT`. |
| A5 | `$LINENO` is unset | empty in oslo, `1` in bash | Every `die() { echo "$0:$LINENO: $*"; }` helper loses its line number. |
| A6 | `$PPID`, `$UID`, `$EUID` are unset | `UNSET` against bash's numbers | `[ "$UID" = 0 ]` is the standard root check in install scripts; unset silently means "not root". |
| A7 | No builtin `printf` | resolves to coreutils | An initramfs `/bin/sh` runs before coreutils is on the filesystem. bash, dash and busybox all build it in. |

`$RANDOM` and `$SECONDS` are bash-isms rather than POSIX, and are listed here only so the decision
to skip them is on the record.

### Round A status — done

All seven closed and verified against bash. Re-running the ~60-idiom probe now leaves one
difference, `$0` under `-c`, which is the two shells reporting their own names.

* **A1** `-n` parses without executing, checked per command so `set -n` mid-script also stops, and
  ignored when interactive per POSIX.
* **A2** `set -a` exports, decided in `set_var` so `read`, `for` variables and `${v:=}` are all
  covered; `set +a` stops it.
* **A3** EXIT traps run in all four subshell forms — `( )`, `$( )`, a pipeline stage, `&` — each
  of which had its own `process::exit`.
* **A4** `$(trap)` reports inherited traps. A subshell keeps the trap *strings* for listing while
  resetting the *actions*, which is the distinction POSIX's command-substitution carve-out needs.
  The listing also matches bash's spelling in both modes: `SIGINT` normally, bare `INT` under
  `set -o posix` — following only the first broke every posix-mode script, and the corpus caught it.
* **A5** `$LINENO`, from brush's own source spans through a `line` on `ListItem`. Matches bash in
  functions, loops and `if` branches. Position is excluded from AST equality: where a command was
  written is not part of what it is, and re-emitting a script legitimately moves it.
* **A6** `$UID`/`$EUID`/`$PPID`, unexported like bash, with an inherited value still winning.
* **A7** `printf` is built in — 31 differential cases against bash at zero differences, including
  `%b` vs `%s` escape handling, `%q` in bash's backslash form, C's `e+03` exponent, and length
  modifiers (`%zb` is `%b`, not an error).

Found while doing it, still open: bash lists signals that were **ignored on entry** (`trap` shows
`trap -- '' INT` for a shell started with SIGINT already ignored); oslo does not track the
disposition it inherited, so it lists nothing for those.

## Round B — Lua is not yet a language you can write the system in

oslo is a Lua shell, and this is where it is furthest from that claim. The whole API is eight
functions: `exec`, `get_var`, `set_var`, `get_pwd`, `get_alias`, `set_alias`, `set_prompt`,
`register_builtin`.

| # | Gap | Measured |
|---|---|---|
| B1 | **A Lua script cannot see its arguments.** `arg` is `nil` and `...` is empty | `oslo build.lua one two` → `arg table: nil` |
| B2 | **No way to capture output.** `oslo.exec("echo hi")` returns only a status; the text goes to stdout | returns `0`, prints `hi` |
| B3 | No `oslo.cd`, no cwd control beyond reading it | — |
| B4 | No environment iteration or unset — `get_var`/`set_var` only, one name at a time | — |
| B5 | No exit-status control: a Lua program cannot choose what the shell exits with except via `oslo.exec` | — |
| B6 | No glob, no path helpers; Lua's `io`/`os` are present but a shell's job is filesystem work | `posix` module absent |

B1 and B2 are the two that make Lua unusable for real work. A scripting language that cannot read
`argv` or capture a command's output is a configuration language.

### Round B status — done

All six closed. `arg`/`...` (B1), `oslo.capture` returning `{out, status}` (B2), `oslo.cd` through
the `cd` builtin so `$PWD` agrees (B3), `oslo.env`/`oslo.unset` (B4), `oslo.exit` travelling as a
shell exit rather than a Lua error (B5), and `oslo.glob` (B6).

Two things the work turned up:

* `oslo.capture` has **no `err` field**. It runs the same capture `$(cmd)` does, which leaves
  stderr on the shell's own — so an `err` could only ever be empty, and an always-empty field
  reads as "no diagnostics" rather than "nobody looked". `oslo.capture("cmd 2>&1")` folds them.
  Capturing the two streams separately needs a second pipe through `eval_command_substitution`
  and is the one Lua item left open.
* `#!/usr/bin/env oslo` used to be read as *shell*, so the shebang an oslo Lua script most
  naturally carries sent it to the shell parser. It now decides nothing — it names the shell, not
  the language — and the extension or the text answers. Found by running the README's own example
  rather than by a test, which is why C2 below is worth having.

## Round C — proving it, not asserting it

The differential corpus is 390 scripts and none of them is a distro script. What that buys is
confidence in the *language*; what it does not buy is confidence in the *job*.

* C1 — corpus cases for every Round A defect, so each closes by deleting a line from
  `tests/differential/expected_fail.rs`.
* C2 — a Lua corpus. There is no differential harness for Lua because there is no second Lua shell
  to compare against; the oracle has to be expected-output files instead. Worth building once, not
  per-test.
* C3 — **run oslo as `/bin/sh` against real scripts.** Symlink it and run a package build, an
  autoconf `configure`, and a set of init scripts. This is the only test that can find what nobody
  thought to write a case for. `sh -n` over every `.sh` on the system is the cheap first version,
  and needs A1 fixed before it can run at all.
* C4 — boot-path realism: behaviour as PID 1 (orphan reaping), behaviour with no `/proc` mounted,
  behaviour when `PATH` is empty.

### Round C status

**C3, the sweep, is the finding.** 745 real shell scripts from this system's store were put through
`bash -n` and `oslo -n`; 5 are not shell at all. Of the 740 bash accepts:

| | before | after |
|---|---|---|
| oslo parses | 658 | **665** |
| rejected — process substitution | 65 | 74 |
| rejected — nesting guard (false positives) | 17 | 1 |

The nesting guard was rejecting real scripts, and the module comment had predicted both causes and
judged them unlikely:

* **A here-document body is data.** `config.guess` — in every autotools project — writes a **C
  program** through `<<EOF`, and its braces counted as 22 phantom unmatched openers. Bodies are now
  stripped before the scan.
* **`$(…)` inside `"…"` is shell again.** A scan that entered `"$(` stayed in double-quote mode and
  read the single-quoted `awk`/`jq` program inside as shell, losing sync at its first `"`. The
  scan now leaves quoted mode for a substitution body and restores it on the closing `)`.

Both fixes are measured against the same input the guard exists for: 40 unmatched `(` is still
refused, in 17 ms. Two bugs in the fixes were caught by tests rather than by the sweep — `<< 2`
read as a heredoc named `2`, and quoting not resumed when a substitution closed in unquoted
context, which an *existing* test caught.

One false positive remains (1 of 740): a generator that builds C source inside a double-quoted
shell string, where the approximate scan desynchronises on the C string literals. It fails safely,
with a syntax error rather than a hang, which is the direction this guard is meant to fail in.

**C4, boot realism.** Empty `PATH` leaves the builtins working; no `HOME`, no `TERM` and no tty are
all fine; a child killed by a signal reports 137 and the shell survives; orphaned grandchildren are
reaped. The shell no longer reads `/proc` anywhere — `times` uses `getrusage(2)` and the prompt's
hostname `gethostname(2)` — so a root without `/proc` mounted is no longer a special case. True
PID-1 behaviour could not be exercised here: `unshare -p` needs privileges this sandbox refuses.

**Process substitution is implemented**, and the sweep is now **739 of 740**. The one rejection
left is the C-source-in-a-quoted-string false positive above.

`<(cmd)` and `>(cmd)` run the body on a pipe and hand over `/dev/fd/N`. Three things decided the
design, each of which is a way it silently fails otherwise: the descriptor has to outlive
*expansion* (the program opens the path only once every word is expanded, so it is closed after
the command, on every exit path); the child must not keep the end the caller was named by, or a
`<(…)` reader never sees EOF and `cat <(echo hi)` hangs; and the descriptor must survive `exec`,
which means clearing the `O_CLOEXEC` Rust sets on every fd it opens.

Verified against bash on 13 forms — both directions, two at once, nesting, redirect targets,
inside `$( )`, a failing body, and `head -1 <(yes)` for the SIGPIPE path. No descriptor leak over
60 substitutions, no zombies.

Known gap: `for f in <(echo a)` is still a parse error. bash accepts a process substitution in a
`for` word list; it did not appear in any of the 740 scripts.

C1 is done (every Round A defect has a corpus case).

**C2, the Lua corpus, is done** — `tests/lua_corpus/`, 10 cases, driven by
`tests/lua_corpus_tests.rs`.

The oracle is the difference that shapes it. The shell corpus can be trusted without anyone
reading it, because bash supplies the expected output; there is no second Lua shell to do that
here, so the expectation is **recorded in the case and written by hand**. Capturing it from a run
would record today's behaviour — bugs included — and then assert it forever, which is worse than
no test because it looks like coverage. A case with no `--[[ expect ]]` block fails for the same
reason.

Writing the expectations first paid for itself immediately: of ten cases, four disagreed with the
shell on the first run, and deciding which side was wrong each time is the point of the exercise.
Three were mine — field splitting means `echo  spaced  out` really does print one space; a
`match("[^:]+$")` keeps its leading space; mlua does not load the `debug` library. The fourth was
a design question worth writing down rather than a bug: `oslo.exec("exit 5")` *ends the script*,
because `exec` runs in this shell — which is the same reason `cd` through it persists.

The second test in the file is the one that matters most. Every case must be **detected** as Lua
without being told, so the corpus exercises the real `oslo case.lua` path rather than forcing
`--lua`. Reintroducing the shipped bug — `#!/usr/bin/env oslo` read as shell — fails it with
"was read as shell, not Lua", which is exactly the bug that reached a release and was found by
running the README example rather than by any test.

## C4 continued — the Alpine VM

`scripts/alpine-vm.sh` boots oslo as an Alpine VM's **PID 1 and `/bin/sh`** and runs
`scripts/alpine-vm-suite.sh` inside it. `--shell` drops to an interactive prompt in there instead.

Alpine because it is musl and busybox: the static release artifact has to run on a system that
shares no libc with the build host, and every utility in the image is a different implementation
from the coreutils the differential corpus was written against. The rootfs *is* the initramfs, so
`/init` is oslo — which is the shortest path to actually being PID 1 rather than simulating it.

24 checks pass in there, including process substitution, `printf` with no coreutils present,
`set -e`, EXIT traps in subshells, `set -a`, `128 + SIGTERM`, and the Lua API capturing `uname`.

It earned its keep on the first boot, with three findings:

* **`$(case …)` does not parse — and it is brush's, not ours.** `echo $(case a in a) echo hit;; esac)`
  is a syntax error while the same body parses standalone, so the first guess was that oslo's own
  word scanner was counting the pattern's `)` as the substitution's closer. Making that scanner
  `case`-aware changed nothing, because brush is the parser and oslo's lexer only re-lexes word text
  *after* it: the diagnostic was brush's all along, and the speculative fix was reverted rather than
  left in as complexity that buys nothing. This joins `for ((;;))` as a limit that needs a grammar
  patch upstream — the second time that has come up, which is the argument for sending both.
* **Process substitution needs `/dev/fd`,** which a minirootfs does not ship. `cat <(echo x)` fails
  with "can't open /dev/fd/3" until `/dev/fd -> /proc/self/fd` exists. bash has the same
  dependency, so the VM's `/init` creates it as a real init would — but it is worth knowing that
  `<(…)` is not available in an environment that has not set that up.
* **PID 1 did not reap reparented orphans — fixed.** A double-forked orphan is reparented to init,
  and only init can reap it; oslo reaped the jobs it started, by pid, and skipped the sweep entirely
  when it had no children of its own — which is exactly an idle init's state. It now calls
  `waitpid(-1, WNOHANG)` at command boundaries, **gated on being process 1**: that is not caution
  about cost but about correctness, since reaping any child in an ordinary shell could consume the
  status a `wait $pid` was about to read. A reaped status that turns out to belong to a known job is
  still recorded, so `$!` and `jobs` stay right if the sweep gets there first. The VM now reports
  `ORPHANS-AT-BOUNDARY:0`.

Two expectations of mine were wrong rather than the shell: `/proc/1/comm` reads `init` because the
kernel takes `comm` from the file named to `execve` — the script — so `/proc/1/exe` is the check
that means anything; and a shell as init cannot reap while blocked in a foreground command, so
that check belongs in `/init` and not in a suite running under it.

## C3 completed — real execution, not just parsing

The 740-script sweep proved *parsing*. Running GNU hello's `./configure` — 25 150 lines of
generated shell — proved execution, and immediately found that oslo could not parse it at all.

The sweep had never covered it. Its file list was `*.sh` plus shebang-detected executables in the
nix store, and autoconf `configure` scripts live in source trees, not there. A number that read as
comprehensive had systematically excluded the most important shell script in the build world.

The cause was the nesting guard again, in three more places. Each was found by instrumenting the
scanner to report *which lines* it still thought were open, rather than by bisecting — bisecting a
file with `head` truncates it mid-construct, so every prefix looks unbalanced and the result means
nothing.

* **A `)` inside double quotes closed a live construct.** `if (eval "test \$(( 1 + 1 )) = 2")` has
  an *escaped* `$`, so the `((` opens nothing while the `))` closed the real `(` from `if (`. Every
  `fi`, `done` and `esac` after that matched against the wrong frame. A `)` in double quotes is now
  never a closer — a genuine `$(` leaves quoted mode, so its closing paren arrives in the unquoted
  branch instead.
* **`<<\_ACEOF` was not recognised as a here-document.** The delimiter can be backslash-quoted as
  well as `'`- or `"`-quoted; autoconf uses that spelling 15 times per `configure`, and those
  bodies are `--help` prose full of the words "do", "if" and "case", each opening a construct that
  never closed.
* **The threshold measured the wrong thing.** `MAX_UNMATCHED_OPENERS` was calibrated against short
  pathological input — `(((((…x` at a prompt — where a few unclosed parens cost seconds of PEG
  backtracking. That risk lives in *small* inputs; a 25 000-line script is not one opener repeated,
  and a parser working through valid text makes progress rather than backtracking. Meanwhile this
  scan is approximate by design, so on a large file the count reflects its own mistakes more than
  the input's. The allowance now grows with input size: strict where the hang was measured,
  generous where the approximation is unreliable and the hang is not realistic.

Result: **`./configure` completes under oslo with exit 0**, producing a working `Makefile` and
`config.status`. `make` then fails identically under oslo and under a bash-configured tree
(`aclocal-1.16` is missing on this machine), which is the controlled comparison that matters.

The sweep is now **740 of 740**, with the last standing false positive gone as well.

A1 and A2 first and alone: A1 is a correctness *and* safety defect, and it is a prerequisite for
C3's cheap sweep. A3–A7 are independent of each other. B1/B2 unblock every other Lua item. C
follows whatever has landed.

## Done: the external oracle, and the terminal paths

Both now run in the Alpine VM, in one invocation, and `scripts/alpine-vm.sh` exits non-zero if
either regresses.

### The oracle: modernish, baked into the image

[modernish](https://github.com/modernish/modernish) is a POSIX-shell library whose initialisation
*is* a battery of named probes for known shell bugs, written against a dozen real shells by someone
with no stake in this one. That is the whole point of using it: every other test here encodes one
reading of POSIX, and twice in this project a hand-written probe was wrong while the shell was
right.

It is fetched on the host and unpacked into `/opt/modernish` at image-assembly time, because the VM
has no network. (`install.sh -n` is **not** a dry run — it wrote 298 files outside the repo when
that was assumed. Not installing at all avoids the question.)

Pointed at oslo, it refused to initialise. Five real defects came out of getting it to run, each
now fixed and covered by a differential-corpus case:

| what it found | why it mattered |
|---|---|
| `command -v if` did not report reserved words | modernish treats this as **fatal** and will not start |
| `${1+"$@"}` joined the arguments and re-split the join on `IFS` | the pre-POSIX argument-forwarding idiom, silently mangled |
| **quoting was dropped from every shell pattern** | `case $x in "$expected")` matched *anything* when `$expected` held a `*` |
| `[[ ]]` field-split and globbed its operands | `[[ $x == "a b" ]]` was `too many arguments`; an empty operand shifted the operator into the operand slot |
| `break` in a loop's *condition* unwound past the loop | took the rest of the enclosing function with it — modernish writes every option parser as `while case … esac; do shift; done` |

The pattern one is the serious defect of the group and was not on anyone's list. Fixing it meant
routing `case`, `[[ ]]` and the `${v#p}` family through the quote-aware matcher pathname expansion
already had, which also retired the `glob` crate: there is now one pattern dialect, not two.

modernish's fatal battery and full initialisation both pass, on musl and busybox, with oslo as
PID 1. Its *regression suite* does not run yet, and the blocker is not oslo: `sys/base/mktemp`
uses `forever do … done`, where `forever` is an alias whose body opens a compound command. oslo
substitutes aliases at execution time, on the command word only, so an alias can replace a command
but cannot contribute syntax. POSIX puts alias substitution in the tokenizer. Real-world POSIX
scripts almost never need this — it is a modernish idiom — so it is recorded rather than scheduled.

### The terminal paths

`scripts/alpine-vm-jobs.sh` runs under `setsid -c` on `/dev/ttyS0`, which is the only way a shell
that is not a REPL still has a *controlling terminal*. Init cannot supply one: the kernel hands
PID 1 `/dev/console`, which can never be claimed as a ctty. It covers process groups under `set -m`,
`jobs`/`bg`, the terminal's foreground process group during and after a foreground job, group
signals, and `wait` statuses — 20 checks, all passing.

Two things it found:

* **`set -m` did nothing in a script.** Job control was enabled only from the REPL, so a script
  that said `set -m` got the half that needs no terminal — separate process groups — while `bg`
  answered `no job control` and left the job stopped for ever. `set -m` now turns job control on
  wherever there is a terminal to claim, as bash does.
* **Three earlier checks were passing vacuously.** busybox's `ps` has neither `-o pgid` nor `-p`,
  so each comparison read the empty string and matched it against another empty string. They read
  `/proc/PID/stat` now, and every reading is asserted non-empty before it is compared.

`scripts/alpine-vm.sh --console` covers the last piece — the *characters*. A `^C` is not a signal a
test can send; it is a byte in the terminal's input queue that the line discipline turns into a
signal for the foreground process group, and only the far end of the line can put it there. With
`-nographic` that far end is the harness's own stdin to qemu, driven through a fifo.

`^C` and `^Z` both pass there, and getting `^Z` to pass was the most instructive hour of the round.

It failed at first, and looked exactly like a shell bug: the job ran to completion as though no
SIGTSTP had been delivered, while `^C` on the same console worked. `stty -a` reported `susp = ^Z`.
Instrumenting the job showed it had its own process group, held the terminal (`tpgid` equal to its
own `pgrp`), and had SIGTSTP at `SIG_DFL` with nothing blocked. Every ingredient was correct.

The cause was the harness. The kernel discards SIGTSTP, SIGTTIN and SIGTTOU sent to an *orphaned*
process group, and `will_become_orphaned_pgrp()` counts a job as orphaned when its parent is PID 1
(`is_global_init(p->real_parent)`). The console harness `exec`d the interactive shell as PID 1, so
every job it started was in such a group. SIGINT is not a stop signal and was unaffected — that
asymmetry was the clue, and it is worth remembering the next time one signal works and another
does not.

Dropping the `exec`, so the shell is a *child* of init, fixes it: `[1]+ Stopped`, status 148, the
job in the table, and the terminal reclaimed afterwards. A real system never has the other shape —
getty, login or sshd always sits between init and a login shell.

### Still open from this round

* `$(case …)` — brush #1052. Workaround: backticks.
* A comment inside `$( )` is only recognised when the number of blanks before its `#` is **even**.
  See "The brush comment bug" below; it is now the thing blocking modernish's regression suite.

## A real distro, not a minirootfs

`scripts/alpine-distro-vm.sh` boots an Alpine userland with **OpenRC** — Alpine's init system,
written almost entirely in POSIX shell by people who have never heard of oslo — with oslo as
`/bin/sh`. `make vm-distro`.

It came up. `openrc sysinit`, `openrc boot` and `openrc default` all returned 0, and the services
that ran are real ones: mounting `/dev/pts` and `/dev/shm`, checking and remounting filesystems,
configuring kernel parameters, creating user login records, cleaning `/tmp`, setting the hostname,
loading modules, setting the clock from hardware, starting syslog. `rc-status` lists 49 services.
OpenRC's whole shell runtime under `/usr/libexec/rc/sh` parses, and sourcing `functions.sh` defines
the helpers every service calls.

**92 shell scripts in the image parse, none rejected** — every `/etc/init.d/*` service, OpenRC's
runtime, and `alpine-conf`'s 22 `setup-*` tools, which are among the densest POSIX shell any distro
ships. The minirootfs VM had found two.

The image is built by layering packages onto the cached minirootfs with `tar` rather than
bootstrapping with `apk`, because `apk` wants to chown and mknod and this runs as an ordinary user
(and user namespaces are restricted on the machine this was written on). Extraction gets the files,
which is all a boot test needs.

Three things about the *test* are worth keeping, because each was a bug in it first:

* The parse sweep **walks the filesystem** instead of using a list of globs. The first version
  missed OpenRC's entire runtime — Alpine had moved it from `/lib/rc/sh` to `/usr/libexec/rc/sh` —
  and reported "all parse" with total confidence.
* The sweep **asserts it is not vacuous**: it fails if it finds fewer than 50 scripts, and it feeds
  `sh -n` a known-bad script and fails if that is accepted. Three checks in the job-control suite
  had already passed by comparing one empty string against another.
* The runlevel check compared against the wrong constant and passed for the wrong reason until the
  first real run.

## Alias substitution, moved ahead of the parser

oslo substituted aliases at *execution* time, replacing a simple command's first word with the
alias body. That is enough for `alias ll='ls -la'` and wrong for everything else, because an alias
body is not a list of arguments — it is **source text**. `alias forever='while :; do'` is a real
idiom, and expanding it after parsing cannot work: by then the `done` at the other end has already
been a syntax error.

[`crate::parser::alias`] now does it where POSIX puts it, on the text before it is parsed, and the
executor no longer expands aliases at all. Doing both would expand twice — `alias ls='ls -F'`
would have become `ls -F -F`.

What the pass has to know, each item learned from something that broke:

* **Command position.** Only the first word of a simple command. Reserved words that introduce a
  command list reopen it; `case` patterns and a `for`/`select` word list do not.
* **Where a word list ends.** A `for` list ends at the `;` or newline before its `do`, but a
  `case` pattern list survives both. Treating them alike swallowed the `DO` in modernish's
  `LOOP for i in 1 to 10; DO … DONE`, which then left the list open for the rest of the file.
* **What is not command text.** `$(( … ))` is arithmetic and `${ … }` is a parameter expansion;
  neither holds commands. Scanning into them turned `$(( n + 1 ))` into `$(( echo BAD + 1 ))` for
  anyone with an alias called `n`. `$( … )` *is* shell text, and aliases do apply there.
* **Definitions in the text itself.** A script may define an alias and use it further down, which
  bash allows by parsing one command at a time. oslo parses a whole unit at once, so the scanner
  reads `alias name=value` as it walks and honours it from the *following* line — which is exactly
  where bash starts honouring it, and why `alias x=y; x` finds no `x` in either shell.
* **Here-document bodies are data**, as ever.

### What it unblocked, and what it found

modernish's `sys/base/mktemp` — the module whose retry loop opens with `forever do` — now loads,
as do `var/arith` and full initialisation. Getting there turned up a **seventh** defect: `let` did
not accept `--` as an end-of-options marker, so every arithmetic test modernish makes (it aliases
`let` to `let --`, precisely so an expression may begin with a minus) died as an unparseable
expression. That bug was invisible until aliases started working.

Three earlier corpus tests had to change: they used `alias e='…'; e` on one line, which bash
answers with `e: command not found`. They passed only because oslo expanded aliases after parsing.

modernish's *regression suite* still does not complete — it now fails much deeper, in modernish's
own signal-name cache (`_Msh_sigCache: unbound variable`). That is progress rather than a
regression: with the pass disabled, modernish no longer initialises at all.

## Chasing modernish deeper: SIGPIPE, and three bugs in the alias pass

Following `_Msh_sigCache: unbound variable` — the wall the alias work left modernish at — took the
oracle several modules further and turned up four more defects. Three of them were in the alias
pass itself, which is worth stating plainly: it shipped with them.

* **A command substitution was substituted twice.** Its body is kept as source in the AST and
  parsed *again*, through the same pass, when it runs. modernish's `alias let='let --'` therefore
  became `let -- --`, and every arithmetic test it makes died. `$( … )` and backquotes are now
  copied through untouched and left for their own parse; the aliases in them still apply, at the
  point where the body is actually parsed.
* **…and a multi-line one was substituted anyway**, because the copy stopped at the end of the
  line. The rest of the body was then scanned as ordinary text. The copy is resumable across lines
  now, which is what a `$( … )` spanning six lines — as modernish's signal table does — requires.
* **A trailing backslash panicked the shell.** `balanced_end` pushed its index one past the end and
  the caller sliced with it. A shell may not panic; the index is bounded and the shapes that
  triggered it are in the tests.

The fourth is oslo's, and it is the serious one:

* **SIGPIPE was ignored.** The Rust runtime sets `SIG_IGN` before `main` so that a write to a
  closed pipe surfaces as `EPIPE` rather than killing the process. For a shell that is a hang:
  `oslo -c 'while :; do echo x; done' | head -1` **ran for ever**, where bash exits at once, and
  `… | head` is one of the most common things anyone types. It also made `kill -s PIPE $$` a no-op.
  Children already had `SIG_DFL` restored, which is why `yes | head` worked and hid it.

  The fix is in the *binary*, not the library: the test binary links the library and would
  otherwise arm itself to die on any write to a closed pipe.

  modernish detects this exactly (`WRN_NOSIGPIPE`) and refuses to load `var/loop` on such a shell,
  warning that "a pipeline such as `foo | head -n 10` never ends". It was right.

### Where the suite stands now

modernish initialises, loads its modules, builds its signal table and gets into `var/loop`. The
regression suite is now blocked by **brush's one-blank comment bug** — `adj/cap/BUG_TRAPFNEXI.sh`
line 7 is `\t# Store this subshell's PID in $REPLY.`, a comment indented by exactly one tab
containing an apostrophe, inside `$( )`. That is the second file in the same codebase to hit it,
which is a good deal more evidence than the 92-script distro sweep produced, and it moves that gap
from "unquantified" to "the next thing in the way".

Still open and unfixed: `printf`/`echo` do not report write errors (`printf x > /dev/full` exits 0
where bash exits 1). modernish calls this `BUG_PUTIOERR`.

## The brush comment bug, minimised

The gap that now blocks modernish's regression suite, reduced to seven bytes:

```sh
$( #'
)
```

bash accepts it. brush-parser 0.4.0 answers `unterminated single quote at 1,5`.

**The rule is parity, not "one blank".** Inside `$( … )`, a comment is recognised only when the
number of blanks before its `#` is *even* — zero and two work, one and three do not, and tabs and
spaces count alike:

| blanks before `#` | 0 | 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|---|
| parses | yes | **no** | yes | **no** | yes | **no** |

The comment also has to contain an *odd* number of quotes, or an unbalanced `(`, `)` or backquote —
`# it's the shell's` has two apostrophes and they pair up harmlessly. That is a trap when writing a
test for it, and it caught the first version of the corpus case here.

Only `$( … )` is affected. The same comment at top level, or inside backquotes, is fine.

**Why.** `consume_nested_construct` tokenises the body with `include_space: true`. In that mode the
blank branch *appends* to the token when none is started and *delimits* when one is, so blanks
alternate; after an odd number a token is in progress. The `#` arm sits after the
"we have a token in progress" arm, so it never runs, and the comment's text is tokenised as shell:

```rust
// brush-parser-0.4.0/src/tokenizer.rs, next_token_until()
} else if state.unquoted() && is_blank(c) {
    if state.started_token() {
        result = state.delimit_current_token(...)?;   // even blank: closes the token
    } else if include_space {
        state.append_char(c);                         // odd blank: *starts* one
    }
    ...
}
...
else if !state.token_is_operator && (state.started_token() || ...) {
    state.append_char(c);                             // the `#` lands here
} else if c == '#' {                                  // ...so this never runs
```

**A reproducer with no oslo in it** — `brush-parser` as the only dependency — is kept in the
scratchpad rather than the repo, since it is not oslo's code:

```rust
let opts = brush_parser::ParserOptions::default();
let mut parser = brush_parser::Parser::new(Cursor::new("$( #'\n)\n".as_bytes()), &opts);
assert!(parser.parse_program().is_ok());   // fails on 0.4.0
```

**Fixed upstream**, by [reubeno/brush#1253](https://github.com/reubeno/brush/pull/1253): blanks
accumulated by `include_space` are there to reproduce the construct's original text, so a `#`
following nothing but those blanks still begins a comment. One clause on the existing condition,
narrowing the guard rather than reordering the arms, because `started_token()` is doing real work
there (`a#b` is one word).

Until that lands in a release, `Cargo.toml` points `brush-parser` at the fork branch carrying it.
The ratchet did its job the moment the dependency moved: `comment_in_command_substitution.sh`
started matching bash and the suite failed until its `EXPECTED_FAIL` line was deleted.

## On the fork, and what it opened up

`brush-parser` is pinned to `bresilla/brush` branch `fix/comment-after-odd-blanks-in-subst` until
PR #1253 lands in a release. That branch is upstream `main` plus the one commit. Nothing in
`src/parser/brush_adapter/` needed changing, `make verify` is green, and both VMs still pass.

Worth knowing: upstream `main` does **not** fix the other two gaps. `$(case …)` (brush #1052) and
unspaced `for ((;;))` both still fail, so their `EXPECTED_FAIL` rows stay.

With the parser fixed, modernish's regression suite now *starts* — it prints its banner and begins
sourcing test files, where before it died in the capability probes. Getting that far turned up one
more defect in oslo's alias pass:

* **A word before a non-empty subshell was read as a function definition.** The check for
  `name ()` only required the `(`, so `not (readonly foo; …)` looked like a definition of a
  function called `not` and the alias was left alone — leaving text that is a syntax error until
  it expands, which is what bash reports for the raw file too. A function definition needs an
  *empty* paren pair; requiring the `)` fixes it. modernish aliases `not` to `! `.

The suite now stops at `builtin.t` with a syntax error at end of input. That one is *not* the
alias pass: substituting the file with modernish's real 15-alias table produces text that parses.
It is something in how modernish preprocesses a test file before sourcing it — its
`_Msh_tmp_doHashbangPreload` rewrites the `#! use …` headers — and that is where the next round
should start.

## Out of scope, deliberately

Process substitution, `coproc` and `select` stay refused-by-name — none appears in POSIX `sh`, and
a distro's scripts cannot rely on them. `$RANDOM`/`$SECONDS` as above.
