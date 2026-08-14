# One shell, several histories

A profile is a database file, named by `$OSLO_PROFILE`. It exists because more than one thing runs
commands through this shell, and an agent that shells out writes thousands of lines into the
history a person is trying to search.

<!-- demo:begin -->
[![profiles-and-histories demo](https://asciinema.org/a/1262746.svg)](https://asciinema.org/a/1262746)
<!-- demo:end -->

## How it works

There is no registry, no state file and no command that creates a profile. The variable names one,
the name becomes a file name, and the directory listing is the only authoritative answer to "which
profiles are there".

```
export OSLO_PROFILE=claude
        │
        │  read on every call, never cached — exporting it mid-session takes
        ▼  effect on the next shell without anything having to be told
profile::current()
        │
        ├─ not a valid name?  complain once on stderr, carry on as "default"
        ├─ empty or unset?    "default"
        ▼
profile::store_path(xdg_data, home, ext)
        │
        └─ $XDG_DATA_HOME/oslo/history/claude/   (or ~/.local/share/…), created 0700
             ├── hist.db          events + projection + directories + run rows   0600
             ├── hist.lock        held while the store is being opened
             └── hist.model       the predictor's snapshot — `oslo` only, never written by
                                  `oslo-minimal`, which has no model
```

**A directory per profile.** These three used to sit flat in `<data>/oslo/` as `claude.kv`,
`claude.kv.lock` and `claude.model`, beside `plugins/` and `direnv/` — so six profiles meant fifteen
files in one directory, and every new per-profile artifact multiplied by the number of profiles. A
directory makes a profile something you can copy to another machine or delete outright, which
`rm claude.*` only approximated. The file names no longer repeat the profile, because the directory
already says it.

**A name is a letter, then letters, digits, `_` or `-`, and at most 64 characters.** Nothing looser,
and a bad name is refused rather than cleaned: the name *is* the file, so escaping a hostile value
would be a thing that can be got wrong, whereas a name that cannot contain a separator cannot name a
file outside the directory however it is handled. `../escape` does not become a file; it becomes one
line on stderr and the default profile.

Starting a shell is what creates the store, so `OSLO_PROFILE=claude oslo` twice accumulates rather
than starting over. It is a profile, not a lock — two shells can share one.

### Why the flag went

There was a `--profile=NAME` flag. It is gone, and `src/cli/tests.rs` has a test
(`the_profile_flag_is_gone`) that keeps it gone: `--profile=claude` is now `invalid option` and
status 2 rather than a silently accepted no-op, because a shell that took the flag and then wrote
to the default store would mix an agent's history into yours without ever saying so.

The reason is inheritance. **A profile is a property of a session, not of one command.** Export the
variable once and every `oslo` that anything spawns inherits it, which is the whole point when the
thing spawning shells is an agent running thousands of commands. A flag covers only the invocation
you remembered to put it on — and you do not get to put a flag on the shells a tool spawns for
itself.

### What the directory is for

`profile::available()` lists the directories under `<data>/oslo/history/` that contain a `hist.db`,
sorts, and adds the current profile if it is not there yet — a brand-new shell would otherwise have
nothing to switch away from. A directory without a store is skipped, so the store is what makes a
profile visible: a directory somebody made by hand has nothing the finder could rank.

That listing is what **Tab in the history finder** walks. Tab moves to the next profile, wrapping,
and does nothing at all when there is only one. The query and the scope survive the switch — you
are asking the same question of a different history — and the bar's right end says which:
`claude @ [global] || 3/57`.

### What it isolates, and what it does not

```
┌─ per profile ─────────────────────┐   ┌─ one copy, whatever the profile ─────┐
│ history/<name>/hist.db            │   │ ~/.config/oslo/config.lua            │
│   history events (the finder)     │   │ $HISTFILE, if you set one            │
│   directory and run rows          │   │   (Up arrow, the `history` builtin)  │
│   what `cd NAME` can jump to      │   │ oslo/universal                       │
│   what Tab and the ghost recall   │   │ oslo/direnv/{allow,deny}             │
│   how Tab ranks a command         │   │ $PATH, aliases, functions, env       │
│ history/<name>/hist.model         │   │                                      │
│   prediction and repair           │   │                                      │
└───────────────────────────────────┘   └──────────────────────────────────────┘
```

**A profile is a store, not a sandbox.** Nothing about the environment changes.

**There is no history file unless you ask for one**, so by default a profile isolates everything a
shell remembers. `$HISTFILE` — or `oslo.history.file` — is an *export* for other programs, written
and never read back; the Up arrow, the finder, the `history` builtin and Tab's ranking all come out
of the profile's own `hist.db`. Two profiles pointed at one `$HISTFILE` therefore share that file
and nothing else.

Command ranking used to be the leak instead. It lived in `~/.oslo_frecency`, one file for every
profile, so an agent's `cd`s stayed out of yours while every command it completed went into the table
that ranks yours. It is read out of the profile's own store now.

### Between machines

Every recorded line is an event: a random 32-byte id, a revision, a 16-byte tie-breaker, the host
and the session. `oslo history sync` merges two database files **both ways** and picks winners from
that stamp — higher revision first, then a tombstone over a live row at the same revision
(`deletion_wins_a_same_revision_conflict`), then the tie-breaker. The argument order chooses
nothing: the two paths are canonicalised and sorted, so both orders run the same operation, and
syncing again reports unchanged rather than duplicating.

Deleting is what makes this work. `oslo history delete` and `oslo history clear` write tombstones
rather than erasing rows, which is what stops the other machine putting them back. `oslo history
prune` is the opposite by design — local retention only, no tombstones, syncs nothing away, because
trimming this machine's copy is not a decision about anybody else's.

An arriving event is then *projected* onto the local aggregate: a history row, its outcomes, and a
run row credited to the directory it ran in. That directory did not exist here, so one is inserted
marked `remote` with the origin host — and `put_dir` deliberately skips the by-path and by-base
indexes for a remote row. **A directory that only exists on another machine can never become a `cd`
target here**, while the command it ran still counts.

### Syncing over ssh, in one command

`oslo history sync` takes two *files*, which is the right primitive and the wrong ergonomics for two
machines. `oslo profile sync` is the transport over it:

```sh
oslo profile key init                              # once, on the machine that has the history
oslo profile export | ssh laptop oslo profile import   # once, to say these two are one profile
oslo profile sync laptop                           # from then on
```

```text
oslo profile fingerprint NAME          ssh laptop oslo profile fingerprint NAME
           └──────────────── must be equal, or nothing moves ────────────┘

ssh laptop oslo profile send NAME  ─────────────►  a snapshot of theirs
           sync_files(mine, theirs)                merges *both* files
ssh laptop oslo profile receive NAME  ◄─────────   the merged copy, merged again over there
```

**The far end is oslo, not `scp`.** A store is a live database, and copying the file under a shell
that is writing to it is how you get half a transaction. `send` takes a proper snapshot with
`backup_to`; `receive` *merges* rather than replaces, so a command typed on the other machine
between the two steps survives instead of being overwritten. Running it twice moves nothing the
second time, which is what makes it safe in a login file or a cron line.

`$OSLO_SSH` replaces the `ssh` it runs — a wrapper, a jump host, an alternate config — and
`$OSLO_SSH_REMOTE_BIN` names the far end's `oslo` when it is not on the default `$PATH`.

### Why a key, and not just the name

`default` here and `default` on a machine you have an account on are two histories that share a
word. Syncing on the strength of the word would merge a stranger's commands into yours, so a profile
carries a **key** and sync refuses unless both ends hold the same one:

```
default: this machine is 4a2705fd014bc22b and laptop is 91c3e0a7715fe2b8 — they are not the
same profile.
  If they should be: `oslo profile export default | ssh laptop oslo profile import default`
```

The key is at `$XDG_STATE_HOME/oslo/profiles/<name>.key`, mode `0600`, and never in the store —
the store is the thing that travels, and a key inside it would travel with every copy. What crosses
the wire is a **fingerprint**: sixteen hex characters of a hash, enough to compare by eye and
useless to anybody who intercepts it.

**It is not the security of the sync.** ssh is already doing that, and is doing it before any of
this is consulted. What the key answers is *identity* — which history is this.

That same key is what [secrets](secrets.md) derive their store key from, so the one export carries
both: the history and the values.

## What makes it different

In bash and zsh, separating one stream of history from another means pointing `$HISTFILE` somewhere
else, and what you get is a second text file of command lines — there is no ranking data to
separate, because there is none. oslo's profile is a whole store: the events, the directory table
and the frecency rows that decide what `cd` and Tab offer all move with it, which is the part that
actually matters when the thing writing is an agent. The mechanism is also the same shape as fish's
`$fish_history`, which selects a history by name rather than by path.

The other difference is a deliberate absence: oslo will not take a flag for this, on the grounds
that the invocations you most want covered are the ones you never type.

## Configuration

```sh
export OSLO_PROFILE=claude        # ~/.local/share/oslo/history/claude/
```

Export it where the agent runs, not in your own shell. That is the whole of it: the profile decides
every store the shell reads and writes.

If you also export a `$HISTFILE` for other programs to read, give each profile its own — a file is a
file and knows nothing about profiles:

```sh
export OSLO_PROFILE=claude
export HISTFILE="$HOME/.oslo_history.claude"
```

`HISTFILE=""` (or `HISTSIZE=0`) is the switch that means "no trace", and it covers the profile
store and the model as well: a session that keeps no history opens neither.

From Lua, the profile arrives on the `pre-record` hook as a field, which is the supported way to
decide what gets written on the strength of who is writing it:

```lua
oslo.on.pre_record(function(c)
  if c.profile == "claude" and c.status ~= 0 then return false end
  return nil
end)
```

There is no `oslo.profile` setting. Reading it back elsewhere is `oslo.env.get("OSLO_PROFILE")`,
which answers nil in the default profile — the default is the *absence* of the variable, not a
value of it.

## Measurements

Each profile is a whole database file, and that has a floor. Pinned by
`a_small_store_stays_small_and_a_large_one_takes_a_whole_step` in
`crates/oslo-base/src/track/kv/tests.rs`: 400 run rows fit inside the 128 KiB a fresh store is born
with, and past that the file steps to 8 MiB in one go. There is no `VACUUM`; the step never comes
back.

The store directory on the machine this was written on shows exactly that shape:

| file | bytes |
|---|---:|
| `history/default/hist.db` | 8,519,680 |
| `history/demo/hist.db` | 8,519,680 |
| `history/padcheck/hist.db` | 131,072 |
| `history/default/hist.model` | 27,617 |

A profile you use once costs 128 KiB. A profile you work in costs 8.5 MB for the life of the
machine, which is the argument for naming profiles after *roles* rather than per task or per day.

## What it cannot do

- **Sync needs `oslo` on the other machine**, on a `$PATH` ssh can see. A bare `scp` of the store
  would be the alternative and is not offered: copying a live database is how you get half of one.
- **Nothing is scheduled.** `oslo profile sync` is a command; putting it in a login file or a timer
  is yours to decide, and it is safe there because a second run moves nothing.
- **The key is not rotated, and there is no revocation.** A machine that has it can read what it
  already has; taking it off the list is `rm` on that machine, not something this can reach.

- **Isolate anything but the store.** Not the environment, not aliases, not `$PATH`, not direnv's
  allow list, not universal variables, and not `$HISTFILE` unless you set it yourself.
- **Delete from the profile you are looking at.** The history finder's Delete acts on
  `track::store()` — this shell's own store — so pressing it after Tab-ing into another profile
  removes from yours, not from the one on screen. The row still disappears from the list, because
  the in-memory copy is dropped either way.
- **Be created or listed by a command.** `oslo profile` is a stub: it prints its help and says the
  subcommands are not written yet. Creating a profile is starting a shell with the variable set;
  listing them is looking at the directory (or pressing Tab in the finder).
- **Tell events apart after a sync.** An event carries a host and a session, never a profile, and
  `oslo history sync` takes file paths rather than profile names. Merging an agent's store into
  yours works, and is a one-way door.
- **Rename or move a profile.** The name is the directory name, so `mv history/claude history/agent`
  is the whole operation — with no shell attached to either. It takes the model with it, which is the
  one thing the flat layout made easy to forget.
- **Stop a shell from writing to the wrong one.** `$OSLO_PROFILE` is inherited like any variable, so
  a shell you open *from* an agent's shell is still the agent's profile until you unset it.

## Where it lives

| path | what |
|---|---|
| `crates/oslo-base/src/track/profile/key.rs` | the key, the fingerprint, and where neither of them goes |
| `src/cli/profile.rs` | `list`, `show`, `key`, `export`, `import`, `fingerprint` |
| `src/cli/profile/sync.rs` | `sync`, and the `send`/`receive` halves the far end runs |
| `crates/oslo-base/src/track/profile.rs` | `ENV`, `current`, `valid`, `store_path`, `profile_dir`, `history_dir`, `available`, `after` |
| `crates/oslo-base/src/track/mod.rs` | `default_path` — the `hist.db` of the current profile |
| `crates/oslo-base/src/predict/mod.rs` | `default_path` — the `hist.model` of the same profile |
| `crates/oslo-base/src/track/sync.rs` | `HistoryEvent`, `EventId`, `preferred`, the codecs |
| `crates/oslo-base/src/track/sync/admin.rs` | `sync_files`, `reconcile`, tombstones, import |
| `crates/oslo-base/src/track/sync/projection.rs` | `apply_event`, imported remote directories |
| `crates/oslo-base/src/track/kv/file.rs` | `prepare_directory` 0700, `make_private` 0600, the lock |
| `crates/oslo-ui/src/finder/run.rs` | `load_profile`, `next_profile` — Tab in the finder |
| `src/cli/history/admin.rs` | `oslo history sync`, `delete`, `clear`, `prune` |
| `src/cli/tools.rs` | the `profile` tool, still a stub |
| `src/cli/help.rs` | `ENVIRONMENT` — where `$OSLO_PROFILE` is documented in `--help` |
