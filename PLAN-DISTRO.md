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

### Round A status

Done and verified against bash: **A1** (`-n` now parses without executing, per command so
`set -n` mid-script also stops, ignored when interactive per POSIX), **A2** (`set -a` exports,
decided in `set_var` so `read`/`for`/`${v:=}` are covered, and `set +a` stops it), **A3** (EXIT
traps now run in all four subshell forms: `( )`, `$( )`, a pipeline stage, and `&`), **A6**
(`$UID`/`$EUID`/`$PPID`, unexported like bash, and an inherited value still wins).

Still open: **A4**, **A5** (`$LINENO` needs line numbers on the AST, the largest of these),
**A7** (builtin `printf`).

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

## Sequencing

A1 and A2 first and alone: A1 is a correctness *and* safety defect, and it is a prerequisite for
C3's cheap sweep. A3–A7 are independent of each other. B1/B2 unblock every other Lua item. C
follows whatever has landed.

## Out of scope, deliberately

Process substitution, `coproc` and `select` stay refused-by-name — none appears in POSIX `sh`, and
a distro's scripts cannot rely on them. `$RANDOM`/`$SECONDS` as above.
