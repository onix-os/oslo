# Making oslo the system `/bin/sh`

Every command here has been run on a Debian-family machine and its output checked. The order
matters in two places, and both are called out.

Read the last section first if you are in a hurry: it is the way back, and knowing it is what makes
the rest safe to try.

## Before you start

**Open a second terminal and leave a root shell in it.**

```sh
sudo -i
```

Do not close it until you are satisfied. If `/bin/sh` breaks, that shell is how you fix it —
`sudo` itself needs to run things, and a machine with a broken `/bin/sh` and no open root shell is
a machine you repair from a rescue USB.

## 1. Install the binary somewhere permanent

```sh
make build
sudo install -m 755 target/x86_64-unknown-linux-musl/release/oslo /usr/bin/oslo
```

**Not out of `target/`.** A symlink into the build directory is one `cargo clean` away from a
system with no shell. `/usr/bin` is where the diversion below expects it, and the release binary is
static musl — no interpreter, no libraries, nothing to resolve at load time:

```
$ file -b /usr/bin/oslo
ELF 64-bit LSB pie executable, x86-64, static-pie linked
```

That matters more than it looks. A dynamically linked `/bin/sh` that cannot find its libraries is a
machine that cannot run a single script, including the ones that would repair it.

## 2. Point `/bin/sh` at it

On any distribution that did the usrmerge — all the current ones — `/bin` is a symlink to
`usr/bin`, so `/bin/sh` and `/usr/bin/sh` are the same file. There is one thing to change:

```sh
sudo ln -sf oslo /usr/bin/sh
```

The target is **relative** (`oslo`, not `/usr/bin/oslo`), which is how `dash` was linked before it
and what keeps the link valid if the tree is ever mounted elsewhere.

Check the three things that matter:

```sh
ls -l /usr/bin/sh                        # -> oslo
sh -c 'echo alive'                       # alive
sudo apt-get --simulate install hello    # dpkg runs #!/bin/sh
```

The third is the important one. If package management works, you can still undo everything.

## 3. Survive a dash upgrade

`/usr/bin/sh` belongs to the **dash** package:

```
$ dpkg -S /usr/bin/sh
dash: /usr/bin/sh
```

There is no `update-alternatives` entry for it — the symlink is shipped by the package and rewritten
by `dpkg-reconfigure dash`. So the link above works until dash next updates, and then quietly does
not. A **diversion** tells dpkg the path is taken:

```sh
sudo sh -c '
  ln -sf dash /usr/bin/sh &&
  dpkg-divert --divert /usr/bin/sh.distrib --rename --add /usr/bin/sh &&
  ln -sf oslo /usr/bin/sh
'
```

Three steps, and the order is the point:

1. **Put dash's link back.** `--rename` moves whatever is at `/usr/bin/sh` aside, and that should be
   dash's file rather than yours.
2. **Register the diversion.** dash's symlink becomes `/usr/bin/sh.distrib`, and every future dash
   install writes there instead.
3. **Put oslo back.** `/usr/bin/sh` is now yours and dpkg will not touch it.

**One chained command, not three.** Between steps 2 and 3 there is no `/bin/sh` on the system at
all; chaining makes that window microseconds instead of however long it takes you to type. dpkg
warns about exactly this:

```
dpkg-divert: warning: diverting file '/usr/bin/sh' from an Essential package
with rename is dangerous, use --no-rename
```

The danger is in the doing, not in the result. Run it as one line and the end state is sound.

Afterwards:

```
$ ls -l /usr/bin/sh /usr/bin/sh.distrib
/usr/bin/sh          -> oslo
/usr/bin/sh.distrib  -> dash
$ dpkg-divert --list | grep /usr/bin/sh
local diversion of /usr/bin/sh to /usr/bin/sh.distrib
$ dpkg --audit          # empty: no package is unhappy about it
```

## What to check afterwards

The things that actually exercise `/bin/sh`, none of which a test suite here reaches:

```sh
python3 -c 'import os; os.system("echo ok")'   # system(3)
make -s -C /tmp all                            # build tools shell out constantly
sudo apt-get --simulate install hello          # maintainer scripts
```

`system(3)` deserves its own line. musl's implementation calls
`execl("/bin/sh", "sh", "-c", "--", cmd, 0)` — with a `--` before the command string — and oslo read
that `--` as the program text until it was pointed at `/bin/sh` and every `os.system()` on the
machine broke at once. It is fixed, and it is the example worth remembering: **the shell's own test
suite cannot reach the paths that other programs use to call it.**

## The way back

From the root shell, instantly, without unwinding anything:

```sh
ln -sf sh.distrib /usr/bin/sh
```

To remove the diversion properly — note the `rm` first, because `--remove` moves `sh.distrib` back
to `sh` and will not overwrite what is there:

```sh
sudo rm /usr/bin/sh
sudo dpkg-divert --rename --remove /usr/bin/sh
```

And without a diversion, the plain undo is:

```sh
sudo ln -sf dash /usr/bin/sh
```

## Where history goes

`/bin/sh` runs thousands of scripts a day and none of them are commands you typed, so **a script
and a `sh -c` record nothing**. Only an interactive prompt writes history; see `--help`'s
ENVIRONMENT section for the variables.

Root is not a special case. The store follows `$HOME`, so root's lands in
`/root/.local/share/oslo/` with the directory `0700` and the file `0600`. Worth confirming once
that your `sudo` agrees:

```sh
sudo printenv HOME
```

`/root` means root's history is its own. If it prints *your* home, root writes into your store as
root and you will hit permission errors on your own history file — fix it with
`Defaults always_set_home` in sudoers, or `OSLO_PROFILE=root` in root's environment.

`scripts/test_root_and_sudo.sh` checks all of that and is read-only.

## What is still untested

Two things this project has no way to exercise, which is worth saying plainly rather than leaving
you to find out:

- **Login and display-manager startup.** `scripts/arch-vm.sh` boots a real Arch userland with oslo
  as PID 1 and `/bin/sh`, which covers a great deal — but not your machine's graphical login.
- **A real `apt upgrade`.** Simulation is not the same as maintainer scripts running as root.

Both will happen on their own soon enough. The root shell in the other terminal is for those.
