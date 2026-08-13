# Secrets

A value kept encrypted on disk and handed out when something asks for it — with the key deliberately
somewhere else, so the store can go where files go.

```sh
oslo secret set stripe                 # asks for the value, masked; or reads standard input
oslo secret get stripe                 # writes it to standard output, and nowhere else
oslo secret list
oslo secret where                      # the two directories, and which of them may be committed
oslo secret rm stripe
```

**`oslo` only**, behind the `secrets` cargo feature. Without it there is no `oslo secret` and `age`
is not compiled, fetched or linked — see [Measurements](#measurements) for what it costs, which is
the reason it is off in `oslo-minimal`.

<!-- demo:begin -->
<!-- demo:end -->

## Why this is a shell's problem

Every secret a person keeps ends up in a shell eventually: exported for one command, pasted into a
`curl`, read out of a `.env` that should never have been written. The shell is where secrets are
*used*, and it is the one program in the chain that can hold a value in memory, hand it to a single
command, and never write it down.

Anything outside the shell has the opposite problem. A secrets manager that is not the shell has to
put the value somewhere the shell can read — an environment variable set at login, a file, a
`source`d fragment — and each of those is the plaintext this was supposed to avoid.

## age, and not a keyring

[age](https://github.com/str4d/rage) is a file encryption format with one key type, no options to
get wrong, and a Rust implementation with no C in it. That last part is not a preference: a shell
that is `/bin/sh` on a static musl system cannot link a C library to read its own secrets.

A system keyring is the obvious alternative and is the wrong shape. It wants a daemon, a session bus
and a desktop — none of which exist on a server, in a container, or in an initramfs, which is
exactly where a shell has to keep working.

## Two directories, with opposite rules

```
$XDG_DATA_HOME/oslo/secrets/NAME.age     mode 0600, encrypted   — safe to commit
$XDG_STATE_HOME/oslo/identity            mode 0600, the key     — never
```

The point of encrypting a store is that the store can then live where files live: a dotfiles
repository, a backup, a synced directory. A key sitting inside it turns that from a feature into the
worst kind of accident — the ciphertext and the thing that opens it, committed together, in one
`git add -A`.

So the key is under `$XDG_STATE_HOME`, a tree whose whole definition is *state a machine keeps to
itself*, and `$OSLO_SECRET_IDENTITY` moves it anywhere else: a USB stick, an encrypted volume,
`~/.ssh` beside the other private keys.

And because a home directory that is itself a repository is a thing people do, every `oslo secret`
checks:

```
oslo secret: the key is inside the git repository at /home/you
oslo secret: move it with $OSLO_SECRET_IDENTITY, or the next commit publishes it
```

**That check was measured, because the first version cried wolf.** `~/.git` on the machine it was
written on is an empty directory left behind by something, and `git -C ~ rev-parse` calls it *not a
git repository* — so an `exists()` test warned about a repository that does not exist. A real one is
a directory with `HEAD` in it, or a *file* saying where the directory is, which is what a worktree
and a submodule have. `is_repository` asks that.

## What happens on a get

```
oslo secret get stripe
   │
   ├─ path("stripe")                     a name is a filename: no `/`, no `..`, no leading `.`
   │                                     — refused, not sanitised
   ├─ read  secrets/stripe.age           the ciphertext
   ├─ read  oslo/identity                the key, or generate one on first use
   ▼
age::Decryptor  ─ x25519 ─► ChaCha20-Poly1305 ─► the value on stdout, with nothing added
```

The value goes to standard output and nowhere else: no cache, no temporary file, no environment
variable set behind your back. `$(oslo secret get stripe)` is the whole interface, and the shell
that runs it decides how long it lives.

Writing goes the other way and both files are written the same careful way — to a scratch file that
is already mode `0600`, then renamed into place. There is no instant at which either the key or a
secret exists and is readable by somebody else, and no half-written file if the machine stops.

The ciphertext is not itself a secret and is still written privately, because what a file *is* is
also information: a size, a modification time, the fact that this name exists at all.

**A trailing newline is dropped on `set`.** The value came from a line somebody typed or from a
`printf` in a script, and a token with `\n` on the end fails authentication in a way that takes an
hour to find.

**At a terminal, `set` asks rather than reads.** Standard input there is the keyboard, so reading it
to end of file means the value is typed in the clear, into the scrollback, and finished with a
Ctrl-D nobody is told about. Instead it is the shell's own masked [`ui input`](userin.md). Piped, it
reads standard input as before — which is what a script needs, and why the value is never a
command-line argument that a history file or a process list could hold.

## The half that makes it a shell feature

A store you have to remember to call is a store you will paste out of. The other half is a stored
[variable](macros.md#a-variable-holds-a-recipe-not-a-value) whose body is the *recipe*:

```sh
oslo macros add --var 'GITHUB_TOKEN=$(oslo secret get gh-token)'
```

Nothing has run yet. The first time something in a shell reads `$GITHUB_TOKEN`, that line is
evaluated — once, in that shell — and from then on it is an ordinary exported variable. A shell that
never mentions the name never decrypts anything.

The same line written as an `export` in `config.lua` decrypts at every shell start, on every
machine, for ever, whether or not anything wanted it. It is the argument this store makes about
files, applied to time.

## Configuration

| | |
|---|---|
| `$XDG_DATA_HOME` | where the store is, under `oslo/secrets`. Falls back to `~/.local/share` |
| `$XDG_STATE_HOME` | where the key is, at `oslo/identity`. Falls back to `~/.local/state` |
| `$OSLO_SECRET_IDENTITY` | the key file, absolutely — checked first, and wins over both |

There is no `config.lua` for this. A path read from a configuration file would be a path read *after*
Lua has run, and the point of the key's location is that it is decided by the environment the shell
was started in.

## Measurements

What the feature costs to carry, on the real binary at the release profile:

```text
  nothing else on          5,467,072 bytes
  with it, as published    5,827,616 bytes    +352 KB, 109 crates
  with it, trimmed         5,618,656 bytes    +148 KB,  36 crates
```

Most of what `age` costs as it comes is not encryption: the error messages are localised through
Fluent, and the recipient types for hardware keys and post-quantum bring `hpke`, `ml-kem`, `p256`
and the elliptic-curve tower beneath them. None of that is optional upstream, so the crate is
vendored and cut down — `vendor/README.md` lists every change. What is left is the age format:
x25519 and passphrase recipients, the header, the STREAM payload.

What a read costs, interleaved, min of five runs of three hundred, against the same binary doing a
`secret list` — which starts the same process and touches the same directory but opens no key and
decrypts nothing:

| | |
|---|---|
| `oslo secret list` | 628 µs |
| `oslo secret get` | **829 µs** |

So reading the key, the x25519 exchange and the ChaCha20-Poly1305 payload together are about
**200 µs**, and the rest is starting a process. It is built for size rather than speed like every
other dependency here; at `opt-level = 3` the binary is 8 KB larger, which is not worth it for code
that runs once when a secret is read.

## What it cannot do

- **One recipient: this machine's key.** No sharing a store with a colleague, no encrypting to a
  second key so another machine can read it, no hardware keys, no passphrase recipients on the
  command line. The format supports all of them; nothing here exposes them yet.
- **No `secret run NAME -- cmd`.** Putting one value in one child's environment and nowhere else is
  the right shape and is not built. Until it is, a variable's recipe reaches a program that reads
  the environment *itself* — `gh`, `aws`, `docker` — only once something has read the name in that
  shell.
- **No rotation, and no re-encryption.** Deleting the identity makes every stored value unreadable,
  and there is nothing that walks the store and rewrites it to a new key.
- **The key is protected by the filesystem and nothing else.** Mode `0600`, like
  `~/.ssh/id_ed25519`. Asking for a passphrase on every read is the alternative, and a shell that
  asks fifty times a day teaches you to keep the value somewhere else instead.
- **A value is not hidden from the process that asked for it.** `oslo secret get` writes to standard
  output; if you put that in a variable, it is in that shell's memory, and if you export it, every
  child gets it. That is the shell being a shell.
- **The file is binary age, not ASCII armour.** It is committable, but a diff can only tell you the
  row changed.
- **No `git`-aware anything.** The repository check is one warning about where the *key* is. Nothing
  stages, ignores or commits on your behalf.

## Where it lives

| | |
|---|---|
| `crates/oslo-base/src/secrets.rs` | the store: paths, the identity, `set`, `get`, `names`, `forget` |
| `src/cli/secret.rs` | `set`, `get`, `list`, `rm`, `where`, and the repository warning |
| `vendor/age`, `vendor/age-core` | the format, vendored and cut down |
| `$XDG_DATA_HOME/oslo/secrets/` | `NAME.age`, one file per secret |
| `$XDG_STATE_HOME/oslo/identity` | the key |
