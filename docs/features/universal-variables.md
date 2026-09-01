# Universal variables

```sh
set -U theme dark      # here, and in every other oslo window
set -U                 # what is in the store
set -U -e theme        # gone, everywhere
```

Behind the **`universal`** cargo feature, which a release build has and `oslo-minimal` does not.
Without it there is no `set -U`, nothing looks for the file, and `set` is exactly the `set` POSIX
describes.

A universal variable is set once and seen by every session on the machine, including the ones
already running, and it is still there after a reboot.

## What it is, next to the two things that look like it

oslo already has two mechanisms that cross a boundary and neither is this one. The
[control socket](control-socket.md) *asks another shell* a question — a request and an answer, both
sides live. [`profile sync`](syncing.md) moves data *between machines*, on demand. This is neither:
one value, on one machine, that every session sees without any of them asking each other anything.

## No daemon, and nothing new linked

One file per user — `$XDG_STATE_HOME/oslo/universal` — replaced atomically, and re-read when it has
changed. There is no process in the middle, so there is nothing to start, nothing to fail to start,
and no state that outlives the sessions using it.

`$XDG_STATE_HOME` because that is what this is: not configuration, which a person edits and keeps in
version control, and not data, which `make configs` mirrors with `rsync --delete`.

```
# oslo universal variables, one per line: NAME<TAB>VALUE
theme	dark
greeting	hello\nthere
```

A universal variable becomes an **ordinary shell variable** in each session, so `$theme` is `$theme`
and nothing that reads a variable has to know where it came from. Values are strings, like every
other shell variable; `set -U x a b` stores `a b` rather than inventing a second kind of value for
`$x` to hold.

Scripts see them too. `oslo -c 'echo $theme'` reads the store at startup, because "every running
session" has to include the `-c` a Makefile just started — setting one from a script that cannot
then read it back is the worst of both.

## The failure modes, written down first

This is the one part of the feature where the obvious implementation is subtly wrong, and wrong
silently. So:

**Two shells writing at once.** `rename(2)` makes each write whole — nobody ever reads half a file —
but the second writer's copy wins and the first one's change is gone. That is the trade fish makes
too. Merging per key would be better and is a different feature: it needs a lock or a log, and both
are things that can be left behind by a shell that was killed. A write does re-read the file first,
so a value another session added a *moment* ago is carried forward rather than dropped; the race is
only between two writes that overlap.

**A session that has not looked recently.** Every read revalidates against the file's identity and
size before answering, so "stale" lasts until the next access rather than until something notifies.
A shell inside a long foreground job is exactly that case.

**A file that is corrupt or truncated.** The parse either succeeds whole or is discarded whole, and
a discarded parse **leaves the session's variables exactly as they were**. A store that cannot be
read must never look like a store that was emptied — that is the difference between a bad afternoon
and a lost `$PATH`. A file that is genuinely *gone* does empty it, which is the other side of the
same rule.

## Why a stat and not an inotify watch

The plan called for `inotify`, which the `nix` dependency already provides. What it buys over
revalidating on access is *immediacy without an access*: a status line that redraws the instant
another window changed something, rather than at the next prompt. What it costs is a descriptor in
the event loop, a queue that a shell inside a long job does not drain, and a second path to the same
answer — and the stale-queue case still needs the stat, because a watch nobody is reading is not a
watch.

One stat per prompt is what that immediacy would have saved. If something ever needs to know between
one prompt and the next, the watch goes in beside this rather than instead of it.

## What a session is told

Each prompt, a session reconciles with the file and reports what moved through
[`on-variable-change`](hooks.md) with **`source = "remote"`** and `scope = "stored"`:

```lua
oslo.on("variable-change", function(e)
  if e.source == "remote" and e.name == "theme" then redraw() end
end)
```

`source` is the field this earns. A status line that redraws when the value it shows changed in
another window, and does not redraw for the `x=1` you just typed, cannot tell those apart any other
way. The window that ran `set -U` hears about it as `local`, from the assignment, once — not twice.

**Only what changed is applied.** A universal variable is an ordinary shell variable once it is in a
session, so overwriting all of them every prompt would undo an `x=2` typed a second ago. The store
is where the value lives, not what the session is allowed to be doing with it.

## `set -U` is one branch, and never reaches POSIX `set`

`-U` is not a shell option and its operands are not positional parameters, so putting it through
the POSIX grammar would mean teaching that grammar about a word that is neither. It is read before
`parse_set_args` and returns there; everything below that branch is the `set` every script expects.

| | |
|---|---|
| `set -U` | list them, quoted so a shell could read the listing back |
| `set -U NAME [VALUE...]` | set it, everywhere and here; values join with a space |
| `set -U -e NAME` | erase it, everywhere; status 1 if there was nothing of that name |

`OSLO_UNIVERSAL` names the file, for a test or a project that wants its own.

## Where it lives

`crates/oslo-shell/src/env/universal.rs` is the store; the `set -U` branch is in
`env/builtins/variables/parameters.rs`; the per-prompt sync is in
`crates/oslo-runtime/src/startup/repl/before.rs`, beside the macro refresh it is modelled on.
