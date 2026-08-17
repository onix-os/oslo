# Making oslo the system `/bin/sh`

`/bin/sh` is the shell every package script, every `system()` call and every `#!/bin/sh` file on the
machine runs through. Replacing it is a reversible change, but it is one where a mistake is felt by
everything at once — so this page is as much about the undo and the checks as about the change.

**Read the last section before you start.** Keep a second terminal open with a root shell already
running in it, for the whole of this. If `/bin/sh` is broken, that shell is how you fix it, and
opening a new one may not work.

## What oslo does when it is `sh`

It notices. Invoked under the name `sh`, oslo turns POSIX mode on for itself:

```console
$ ln -s /usr/local/bin/oslo /tmp/sh
$ /tmp/sh -c 'set -o | grep posix'
posix	on
```

That is the whole of the adaptation, and it is what makes this safe to consider at all: the
interactive layer — the ghost suggestion, the correction, the structured pipeline, `rm`'s extra
guards — is off in a `#!/bin/sh` script whether or not you do any of this. A script gets a POSIX
shell. See the option table in the README for the eight extensions this covers.

Startup files follow POSIX too:

| | |
|---|---|
| **login** (`-l`, or `argv[0]` starting `-`) | `/etc/profile`, then `~/.profile` |
| **interactive** | `~/.config/oslo/init.lua` |
| **any** | `$ENV`, last |

`/etc/profile.d` is not walked separately — `/etc/profile` does that itself with `run-parts`, and a
shell that also did it would source every file twice.

## Which binary

A release publishes two per architecture, both static musl, and they differ in the optional
features rather than in the shell:

| | |
|---|---|
| `oslo` | every optional feature — the model, scratches, `direnv`, `argc`, `nix`, secrets |
| `oslo-minimal` | none of them: the shell, the editor, the Lua layer |

**`oslo-minimal` is the one to point `/bin/sh` at** if you are pointing it at anything. It is
smaller, it links less, and none of what it leaves out is reachable from a `#!/bin/sh` script
anyway. Nothing stops you using the full `oslo` as your *own* login shell at the same time; they
are two files.

Install it somewhere that is not `/bin`, so that the shell and the symlink are separate things:

```sh
sudo install -m 0755 oslo-minimal /usr/local/bin/oslo-minimal
```

## Check it before you commit to it

Run the machine's own scripts through it first. `sh -n` parses without running:

```sh
for f in /etc/profile /etc/profile.d/*.sh; do
  /usr/local/bin/oslo-minimal -n "$f" || echo "FAILED: $f"
done
```

And on Debian or Ubuntu, where package maintainer scripts are the ones that matter:

```sh
for f in /var/lib/dpkg/info/*.postinst /var/lib/dpkg/info/*.prerm; do
  head -1 "$f" | grep -q '^#!/bin/sh' || continue
  /usr/local/bin/oslo-minimal -n "$f" || echo "FAILED: $f"
done
```

A failure here is a reason to stop, and to report it — a maintainer script oslo cannot parse is a
bug in oslo, not in the script.

## Debian and Ubuntu: the diversion

`/bin/sh` on Debian is a symlink managed by the `dash` package, and a plain `ln -sf` over it is
undone the next time `dash` is upgraded or reconfigured. `dpkg-divert` is how you take the file out
of the package's hands so the change survives:

```sh
sudo dpkg-divert --divert /bin/sh.distrib --rename /bin/sh
sudo ln -s /usr/local/bin/oslo-minimal /bin/sh
```

The first line renames the existing `/bin/sh` to `/bin/sh.distrib` and records that dpkg should
leave that path alone from now on. The second puts oslo there.

To undo it, in this order:

```sh
sudo rm /bin/sh
sudo dpkg-divert --rename --remove /bin/sh
```

The `--rename` on the way out moves `/bin/sh.distrib` back. Check that `/bin/sh` is a symlink to
`dash` again before you close that root shell.

> The Debian way to make this choice for `dash` itself is
> `sudo dpkg-reconfigure dash`, which asks whether `/bin/sh` should be `dash`. It only ever
> chooses between `dash` and `bash`, which is why oslo needs the diversion instead.

## Other distributions

There is no diversion mechanism on Arch, Fedora or a NixOS-like system; `/bin/sh` is a plain
symlink (Arch, Fedora) or is built into the system generation (NixOS).

- **Arch, Fedora, and most others**: `sudo ln -sf /usr/local/bin/oslo-minimal /bin/sh`, and expect
  a `filesystem` or `bash` package update to put it back. Re-apply after such an upgrade.
- **NixOS**: do not touch `/bin/sh` — it is a symlink into the store and will be rewritten. There is
  no supported way to change it; use oslo as your login shell instead (below).

## Your own shell, which is the smaller change

Making oslo *your* interactive shell touches nobody else's scripts and is undone with one command:

```sh
echo /usr/local/bin/oslo | sudo tee -a /etc/shells
chsh -s /usr/local/bin/oslo
```

A shell must be listed in `/etc/shells` before `chsh` will accept it. To go back:
`chsh -s /bin/bash`.

**This is the recommended arrangement**: the full `oslo` as your login shell, `/bin/sh` left as it
was. You get everything on the feature pages, and no package script's behaviour depends on your
choice of shell.

## If something breaks

Symptoms of a bad `/bin/sh` are unmistakable and immediate: `sudo` may still work, but package
installs fail, `systemctl` units with `ExecStart=/bin/sh -c …` fail, and new terminals may not
open.

From the root shell you kept open:

```sh
rm /bin/sh
dpkg-divert --rename --remove /bin/sh    # Debian/Ubuntu; elsewhere:
ln -sf /bin/dash /bin/sh                 # or /bin/bash, whichever it was
```

If no shell will start at all, boot with `init=/bin/bash` on the kernel command line, remount `/`
read-write with `mount -o remount,rw /`, and run the same commands.
