# The job that will not take a Ctrl-C

Some programs catch `SIGINT` and carry on. A `trap "" INT` in a script, a client retrying through
the interrupt, a tool with a cleanup handler that hangs — and the terminal is yours no longer:

```sh
$ sh -c 'trap "" INT; sleep 300'
^C ^C ^C          nothing, and nothing can
```

Every shell behaves this way, and it is not a bug in any of them. It follows from where the shell is
standing. With this turned on, oslo counts the presses and takes the terminal back:

```lua
oslo.misc.interrupt_escape = 3      -- off by default
```

```
$ sh -c 'trap "" INT; sleep 300'
^C ^C
oslo: press ^C again to take the terminal back
^C
[1]+  Stopped                    sh -c 'trap "" INT; sleep 300'
oslo: sh: stopped after 3 interrupts
$
```

The `[1]+ Stopped` line is the shell's ordinary announcement for a job that stopped — the same one
Ctrl-Z produces, because as far as the rest of the shell is concerned that is what happened. The
line after it is the part that says *why*.

The job is **stopped, not killed**. It is in the job table, and `fg`, `bg` and `kill %1` all mean
what they always meant.

<!-- demo:begin -->
[![interrupt-escape demo](https://asciinema.org/a/1263434.svg)](https://asciinema.org/a/1263434)
<!-- demo:end -->

## Why the shell cannot see your Ctrl-C

This is the whole design, and the rest follows from it.

The terminal driver turns `^C` into a `SIGINT` for **one** process group: the *foreground* one. A
shell doing job control puts each job in a group of its own and hands the terminal to it — that is
what makes Ctrl-C reach `sleep` instead of killing your session, and what lets a job read from the
terminal at all.

```
     terminal ── ^C ──► SIGINT to the foreground group
                            │
   shell (its own group)    │        the job's group
   sitting in waitpid ──────┼──────► sh ── sleep
        ▲                   │
        └── receives nothing, because it is not that group
```

So the shell is asleep in `waitpid` and is never told a key was pressed. Everything oslo knows about
an interrupted command it works out *afterwards*, from the wait status of a child that died. That is
enough for a job that dies. For one that does not, there is nothing to work out.

And no keystroke produces `SIGKILL` — the tty driver cannot send one, by design. So something has to
*observe* the interrupt and act on it, and the only way to observe it is to be inside the group the
kernel is signalling.

## One extra process, and what it is not

oslo forks a **watcher**: a small process that joins whichever group currently owns the terminal and
counts the interrupts it receives.

```
      shell 772304
        │
        ├── 779043  oslo (the watcher)   pgid 779346   ← joined the job's group
        └── 779346  sleep 77             pgid 779346   ← your command, unchanged
```

**It is a sibling, not a wrapper.** Your command's parent is still the shell. Nothing sits between
the two: no extra layer on stdin or stdout, no change to the exit status, no change to what signals
reach what. The watcher reads nothing, writes nothing (except the one notice below), and holds no
terminal.

**One per session, not one per command.** It is forked lazily on the first foreground job and then
repositioned into each new job's group with `setpgid`. The tree above was taken after five earlier
commands; there is still exactly one.

**Nothing at all when it is off.** With `interrupt_escape` unset — the default — the fork never
happens. A script, an `oslo -c`, a session spent in builtins and a shell with no job control all
carry the cost of nothing and behave exactly as they did.

It dies with the shell, through `PR_SET_PDEATHSIG`, so it cannot outlive the session that made it.

## Stopped rather than killed, and why that is the default

`SIGSTOP` is the default action, and it is the one that destroys nothing.

* **It cannot be caught or ignored.** That is the whole point: the programs this exists for are
  precisely the ones that caught the interrupt.
* **`waitpid` already reports it.** The shell's existing Ctrl-Z path takes over — the job is
  recorded, the terminal is reclaimed, the prompt returns. No new code sits on the signal path.
* **No new signal is aimed at the shell**, so there is nothing for a user `trap` to collide with. A
  shell-side handler could have been replaced by `trap … USR1` and the feature would have stopped
  working with nothing to show for it.
* **The decision stays yours.** A killed job is a decision the shell made for you; a stopped one is
  a job you can resume, background, inspect or kill.

A config that wants something else says so:

```lua
oslo.misc.interrupt_escape = { after = 3, action = "kill", notify = false }
```

| `action` | sends | |
|---|---|---|
| `stop` | `SIGSTOP` | the default; the job survives in the job table |
| `kill` | `SIGKILL` | gone, and its whole process group with it |
| `hup` | `SIGHUP` | what a closing terminal sends |
| `quit` | `SIGQUIT` | Ctrl-\'s signal; dumps core where that is enabled |

Whichever it is, the signal goes to the **group**, because a job is its process group — signalling
the leader alone would leave a pipeline's other stages and any grandchildren behind.

## It tells you before it acts

On the press *before* the last:

```
oslo: press ^C again to take the terminal back
```

Two Ctrl-C into a job that is ignoring them is exactly the moment somebody is deciding whether
anything is listening at all, and a feature nobody knows fired is a feature nobody has. Turn it off
with `notify = false`.

That line is written from inside a signal handler, so it is a bare `write(2)` — the notice had to be
something safe to emit there, which ruled out anything that allocates or takes a lock.

## Leaving with a job stopped

```
$ exit
oslo: exit: there are stopped jobs
$ exit
```

bash's rule, and it matters more here. A job you suspended with Ctrl-Z is one you know about; a job
the watcher stopped *for* you is exactly the one you would otherwise walk away from. Repeating the
command is the confirmation.

*(oslo's confirmation lasts until it is used, where bash clears its own on any intervening command.
Matching that needs a per-command counter, and the warning — not the expiry — is the part that stops
somebody leaving without knowing.)*

## From Lua

```lua
oslo.on["on-job-escalated"](function(e)
  -- e.action ("stopped", "killed", "hung up", "quit")
  -- e.presses, e.pgid, e.signal, e.text
  oslo.ui.log("had to " .. e.action .. " " .. e.text)
end)

oslo.job.watcher()
-- { after = 3, action = "stop", notify = true, running = true }
```

`watcher()` reports whether anything is actually **doing** it as well as what was configured, and
the two come apart in exactly the case that matters: a shell with no job control never forks a
watcher, whatever the setting says. A caller reading the setting alone would report a feature that
is not running.

The hook is observation only. By the time it can run, the action has already happened — what acted
was a signal handler in another process — and it is delivered at the next safe point, like every
other notifying hook.

## What it cannot do

**A process wedged in an uninterruptible kernel call is beyond this, and beyond everything else.** A
task blocked inside a syscall — a dead NFS mount, failing hardware — has its signals *recorded* and
delivered when the call returns. That is as true of `SIGKILL` as of `SIGSTOP`. If a command is stuck
in `D` state, nothing in any shell will move it, and it is worth knowing which case you are looking
at:

```sh
ps -o stat= -p <pid>     # D means no signal will land until the kernel is done
```

A large `rm` is usually **not** that case — it runs in `R` and dies of the first Ctrl-C like
anything else. The programs this feature is for are the ones that *chose* to ignore the signal.

**It does not watch background jobs.** The watcher joins the group that owns the terminal, and a
background job does not own it. Ctrl-C was never going to reach one anyway.

**The count is per job, not per unit of time.** Three presses means three during one command; the
counter resets when the next job starts. A window would need a clock and would make the behaviour
depend on how fast you type, and "I have now asked three times and it is still there" is as true
over ten seconds as over one.

## Where it lives

| Path | Key items |
| --- | --- |
| `crates/oslo-shell/src/exec/job/sentinel.rs` | the watcher: `watch`, `stand_down`, `take_events`, the handler |
| `crates/oslo-shell/src/exec/simple/external.rs` | where a foreground job starts it, and `report_escalations` |
| `crates/oslo-ui/src/settings/escape.rs` | `InterruptEscape`, `EscapeAction` |
| `crates/oslo-shell/src/env/builtins/control.rs` | `refuse_over_stopped_jobs` — the `exit` question |
| `tests/interrupt_escalation_tests.rs` | nine cases, on a real pty |

The tests are on a pty because there is nowhere else this exists: the whole mechanism is about which
process group the terminal driver signals, and a pipe has no terminal driver.

See [POSIX, where it counts](posix-fidelity.md) for the ordinary Ctrl-C path this sits on top of,
and [hooks](hooks.md) for what `on-job-escalated` shares with the rest of them.
