# Secrets

A value kept encrypted on disk and handed out when something asks for it — with the key deliberately
somewhere else, so the store can go where files go.

```sh
oslo secret set stripe                 # asks for the value, masked; or reads standard input
oslo secret get stripe                 # writes it to standard output, and nowhere else
oslo secret run TOKEN=stripe -- curl … # one value, one child, no shell in between
oslo secret list
oslo secret where                      # the two directories, and which of them may be committed
oslo secret rm stripe
```

**`oslo` only**, behind the `secrets` cargo feature. Without it there is no `oslo secret`, no
`oslo.secret`, and `age` is not compiled, fetched or linked — see [Measurements](#measurements) for
what it costs, which is the reason it is off in `oslo-minimal`.

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

## A store is three things

A **directory**, an ordered list of **keys** to decrypt with, and a list of **recipients** to
encrypt to. All three are configurable, and there can be more than one store.

```
$XDG_DATA_HOME/oslo/secrets/NAME.age     the `user` store — mode 0600, encrypted
$XDG_DATA_HOME/oslo/stores/work/         a named store
$XDG_DATA_HOME/oslo/plugins/notes.secrets/   a plugin's own, beside the .kv oslo.db makes for it
$XDG_STATE_HOME/oslo/identity            the key
$XDG_STATE_HOME/oslo/secrets.conf        which stores exist, and what each is made of
```

An install that has configured nothing gets one store, one key file and one recipient — byte for
byte the encrypt and decrypt path this had when it had no configuration at all, and one failed
`open(2)` on the read path.

## Configuration is a file, not Lua

`oslo secret get` is dispatched as a tool: it never builds an `Environment`, never reads
`config.lua`, never starts a Lua interpreter. That is not an oversight. `$(oslo secret get
gh-token)` has to work from `dash`, from `cron`, from a `Makefile` and from a container, none of
which have run an oslo config — so configuration that only existed after `config.lua` had run would
apply in your interactive shell and silently not apply anywhere else. For a *recipient list* that
means encrypting to the wrong key and finding out at restore time.

So it is a flat file that the process doing the decrypting reads for itself:

```
# ~/.local/state/oslo/secrets.conf
default work

[user]
key file /home/you/.local/state/oslo/identity

[work]
directory /home/you/src/dotfiles/secrets     # a store meant to be committed
recipient age1ql3z7hjy54pw3hyww5ayyfg7zqgvc7w3j2elw8zmrj2kg5sfn9aqmcac8p   # you
recipient age1lggyhqrw2nlhcxprm67z43rta597azn8gknawjehu9d9dl0jq3yqqvfafg   # a colleague
key file /home/you/.ssh/oslo-work
key command pass show oslo/age-identity
```

**Beside the key, not inside the store.** The store is meant to be committable; this file names this
machine's key paths, and a recipient list published with a store is a disclosure of exactly which
devices can read it. A `secrets.conf` restored from somebody else's backup silently changing who
this machine encrypts to would be recipient injection with nobody having to attack anything.

**A malformed line is an error, never a skipped line** — the failure that prevents is a typo in a
`recipient` leaving a store encrypted to one fewer key than its owner believes, discovered on the
day the other key is the only one left.

Nothing needs to be edited by hand. The commands that write it splice lines rather than re-render
the file, so comments and ordering survive:

```sh
oslo secret key add file ~/.ssh/oslo-work
oslo secret key add command -- pass show oslo/age-identity
oslo secret key init                       # generate this store's key file, explicitly
oslo secret key list

oslo secret recipient add age1lggyhq…
oslo secret recipient add --from RECIPIENTS.txt
oslo secret recipient --export > RECIPIENTS.txt
oslo secret rotate                         # re-encrypt everything to the list as it now stands
```

Adding a recipient does not re-encrypt anything: they can read what is written after today, and
`rotate` is the separate, deliberate step that gives them the rest. It is separate because it
rewrites every file in the store, and because *who could read this before I changed it* is a
question with a permanent answer.

### Which store an invocation means

| | scope |
|---|---|
| `--store NAME` | this invocation |
| `$OSLO_SECRET_STORE` | this shell, inherited by every child — `OSLO_SECRET_STORE=work make deploy` |
| `default NAME` in `secrets.conf` | this machine |
| `user` | built in |

`$OSLO_SECRET_IDENTITY` still names the key file directly and still wins, so a setup that used it
before any of this existed is untouched.

## Keys, including ones oslo cannot compute

A key is a **file** to read, or a **command** to run whose output is the identity. The second exists
because the alternative is compiling every way a person might hold a key into a shell meant to be
`/bin/sh` — a password manager, a smartcard wrapper, a decryption service, whatever they already
use:

```
key command pass show oslo/age-identity
key command gpg --quiet --decrypt /home/you/age-identity.gpg
```

**Native keys are tried first, always.** age stops at the first identity that fits, so a store whose
file key opens the file never runs the program another key source names — no `$PATH` walk, no fork,
and a cron job on a machine that cannot reach the other key degrades instead of hanging on it. A
store listing a file key before a command reads at the same ~829 µs it always did.

What fences the command:

* **argv, never a shell string.** Nothing reaches `/bin/sh`, so there is no quoting layer to get
  wrong and no `$(…)` in a configuration file.
* **Never in a `plugin.*` store.** A plugin's store cannot fork, whichever door the line came
  through — the command refuses to write it, and a hand-edited `secrets.conf` is refused when the
  store is opened.
* **`$OSLO_SECRET_NO_EXEC`**, set to anything non-empty, skips every command source and names it in
  the failure. Exported once by a cron job or a container and inherited by every child, it makes
  *this will not fork* something to assert rather than infer.

### No age plugin client

age reaches hardware keys — YubiKeys and the like — through an external `age-plugin-NAME` binary
speaking a stanza protocol over pipes. **oslo does not speak it.** Supporting it would mean
restoring the client code that the vendored `age` had removed, and this shell does not carry code
for hardware it cannot itself talk to.

The consequence, stated plainly: a `age1yubikey1…` recipient is refused at the moment you add it,
with the reason, rather than accepted and failing on the next write. If your key lives in a device,
`key command` is the route — anything that can print an age identity will do — and if what you want
is the real thing, use `age` itself and keep the file where oslo can read it.

## Several recipients, and a store you can commit

The point of encrypting a store is that the store can then live in a dotfiles repository, a backup,
a synced directory. Several recipients is what makes that useful for more than one machine or more
than one person:

```sh
oslo secret --store work recipient add "$(ssh laptop oslo secret key list)"   # another machine
oslo secret --store work rotate
```

A recipient this binary cannot use is refused when it is written, not when the next `set` fails —
the wrong end of the mistake to find out at.

## What happens on a get

```
oslo secret get stripe
   │
   ├─ read secrets.conf                   which store, which keys, which recipients
   ├─ path("stripe")                      a name is a filename: no `/`, no `..`, no leading `.`
   │                                      — refused, not sanitised
   ├─ read secrets/stripe.age             the ciphertext
   ├─ native keys                         files, in the order written
   │     └─ no match? ─► external keys    only then is a program run
   ▼
age::Decryptor ─ x25519 ─► ChaCha20-Poly1305 ─► the value on stdout, with nothing added
```

The value goes to standard output and nowhere else: no cache, no temporary file, no environment
variable set behind your back. Writing goes the other way and every file — the key, the ciphertext,
`secrets.conf` — is written to a scratch file that is already mode `0600` and then renamed. There is
no instant at which any of them exists and is readable by somebody else, and no half-written file if
the machine stops.

**A trailing newline is dropped on `set`.** The value came from a line somebody typed or from a
`printf` in a script, and a token with `\n` on the end fails authentication in a way that takes an
hour to find.

**At a terminal, `set` asks rather than reads.** Standard input there is the keyboard, so reading it
to end of file means the value is typed in the clear, into the scrollback, and finished with a
Ctrl-D nobody is told about. Instead it is the shell's own masked [`ui input`](userin.md). Piped, it
reads standard input as before — which is what a script needs, and why the value is never a
command-line argument that a history file or a process list could hold.

## The two ways a value reaches a program

```sh
export GITHUB_TOKEN=$(oslo secret get gh-token)      # in this shell, and every child, for good
oslo secret run GITHUB_TOKEN=gh-token -- gh pr list  # in one child, and nowhere else
```

A command substitution puts the value in the *calling shell*, where a `set` prints it and
`/proc/PID/environ` holds it for as long as that shell lives. `run` execs the command directly with
one extra variable and no shell in between, so the value's lifetime is the child's. It is still an
environment, readable by the user who started it — what it buys is that the value is not in anything
else. `VAR=` with no name means the secret named after the variable, lowercased and hyphenated:
`oslo secret run GH_TOKEN= -- gh` reads `gh-token`.

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

## From Lua

```lua
oslo.secret.get("gh-token")            -- the store this shell would use
oslo.secret.set("gh-token", value)
oslo.secret.list()
oslo.secret.stores()

local work = oslo.secret.open("work")  -- another store, by name
work:get("deploy")  work:set("deploy", v)  work:forget("deploy")
work:list()  work:where()

local sealed = oslo.secret.seal("anything")   -- the crypto without the filing
oslo.secret.unseal(sealed)
```

`seal` and `unseal` are base64 of a whole age file, so a plugin can encrypt something it keeps
somewhere else of its own — in `oslo.db`, in a file, over a network. Base64 because an age file is
binary and because turning on age's ASCII armour measured 45 KB for a format nothing on this side of
the boundary would read.

### What a plugin may reach

A plugin gets **its own encrypted store**, unconditionally, with no name to pass because the name is
not the plugin's to write:

```lua
local mine = oslo.secret.mine()
mine:set("cursor", "42")
```

It lives at `$XDG_DATA_HOME/oslo/plugins/<name>.secrets/`, beside the `.kv` that
[`oslo.db`](plugins.md#the-database) already makes for it, so uninstalling stays an `rm -r`. It is
encrypted to *your* recipients with *your* key rather than one of its own: a per-plugin key would
enforce nothing (every plugin can read every file through `oslo.fs`) while adding private keys at
rest and making a lost one a data-loss event the machine's owner cannot recover from. A shell must
never hold a secret its owner cannot open.

**`plugin.` is refused from Lua, always, to every caller**, so `oslo.secret.mine()` is the only door
to a plugin store. The check is on the *name asked for* rather than on who is asking, which is what
makes it hold: it cannot be sidestepped by deferring the call into a hook or a timer. The command
line has no such rule — `oslo secret --store plugin.notes list` works, because you must be able to
see and remove what a plugin kept on your machine.

For **your** secrets, a plugin declares what it will read:

```lua
-- plugin.lua
return {
  name = "gh", version = "0.3.1",
  builtins = { "gh-pr" },
  secrets = { "gh-token", "gh-host" },     -- names, never a wildcard or a prefix
}
```

`oslo plugin install` prints it before you decide to trust it, and it can, because the manifest is
evaluated in a fresh interpreter with no `oslo` global — reading what a plugin claims is not the
moment its code first runs:

```
gh 0.3.1 reserves: gh-pr
  secrets: gh-token, gh-host   it will be able to read these
install and allow it to run? [y/N]
```

A handle acquired while that plugin's file is loading is filtered to those names, and a name it did
not declare is refused with the reason. Its `list()` shows what it could read rather than what is
there, because the names in a store are themselves information.

**This is a disclosure, not a sandbox, and the difference matters.** A plugin can run
`oslo secret get` as a subprocess and nothing here stops it; oslo
[declines to sandbox plugins by name](plugins.md#trust). What the declaration buys is a plugin that
can be *caught contradicting its own manifest* — which is the baseline a review, an audit and any
future sandbox all need. It also only holds while the plugin's file is loading: a handle stashed in
a global outlives the load, and a call made from a hook is indistinguishable from one made at the
prompt.

## Configuration

| | |
|---|---|
| `$XDG_DATA_HOME` | where stores live, under `oslo/`. Falls back to `~/.local/share` |
| `$XDG_STATE_HOME` | where the key and `secrets.conf` live. Falls back to `~/.local/state` |
| `$OSLO_SECRET_IDENTITY` | the `user` store's key file, absolutely — checked first, wins over both |
| `$OSLO_SECRET_CONF` | the configuration file, absolutely |
| `$OSLO_SECRET_STORE` | which store this shell and its children mean |
| `$OSLO_SECRET_NO_EXEC` | skip every key source that would run a program |

There is no `config.lua` for any of this, and that is the point of the section above: a path read
after Lua has run is a path a `cron` job never sees.

## Measurements

What the feature costs to carry, on the real binary at the release profile. First, `age` itself,
measured before anything was designed on top of it:

```text
  nothing else on          5,467,072 bytes
  with it, as published    5,827,616 bytes    +352 KB, 109 crates
  with it, trimmed         5,618,656 bytes    +148 KB,  36 crates
```

Most of what `age` costs as it comes is not encryption: the error messages are localised through
Fluent, and the recipient types for hardware keys and post-quantum bring `hpke`, `ml-kem`, `p256`
and the elliptic-curve tower beneath them. None of that is optional upstream, so the crate is
vendored and cut down — `vendor/README.md` lists every change.

Re-measured on the binary as it now stands, with stores, keys, recipients, `secrets.conf`,
`oslo.secret` and `oslo secret run` in it:

```text
  every other feature on   6,631,840 bytes
  with it                  6,853,056 bytes    +216 KB, still 36 crates
```

So the whole layer above `age` is **68 KB and no new dependency**. `base64` was already in the tree
as an unconditional dependency of vendored `age`.

What a read costs, interleaved, min of five runs of three hundred, against the same binary doing a
`secret list` — which starts the same process and touches the same directory but opens no key and
decrypts nothing:

| | |
|---|---|
| `oslo secret list` | 628 µs |
| `oslo secret get` | **829 µs** |

So reading the key, the x25519 exchange and the ChaCha20-Poly1305 payload together are about
**200 µs**, and the rest is starting a process. With no `secrets.conf` the configuration costs one
failed `open(2)`; with one it is a `read_to_string` of a few hundred bytes and a line parse. It is
built for size rather than speed like every other dependency here; at `opt-level = 3` the binary is
8 KB larger, which is not worth it for code that runs once when a secret is read.

## What it cannot do

- **No age plugins, so no hardware keys directly.** See [above](#no-age-plugin-client): a
  `age1yubikey1…` recipient is refused with its reason, and `key command` is the route to a key oslo
  cannot compute itself.
- **No passphrase recipients, and no `ssh` keys.** The format supports both; nothing here exposes
  them. A passphrase asked on every read is also the thing that teaches people to keep the value
  somewhere else.
- **Nothing here is protected from another plugin.** All Lua runs on one interpreter with one `oslo`
  global, and `oslo.fs` reads any file this user can. A plugin's store is encrypted against the
  *disk*, which is worth having; it is not isolated from other plugins, and saying otherwise would
  be a lie.
- **The declaration is not enforcement.** `oslo.proc.exec("oslo", "secret", "get", …)` walks past
  it, and so does a handle acquired at load and used later.
- **No rotation of the key itself.** `rotate` re-encrypts to the current recipients; there is
  nothing that retires an identity and rewrites every store to a new one. Deleting an identity makes
  everything encrypted only to it unreadable.
- **The key is protected by the filesystem and nothing else.** Mode `0600`, like
  `~/.ssh/id_ed25519`.
- **A value is not hidden from the process that asked for it.** `oslo secret get` writes to standard
  output; if you export it, every child gets it. `run` is the narrower door, and it is still an
  environment.
- **The file is binary age, not ASCII armour.** It is committable, but a diff can only tell you the
  row changed.
- **No `git`-aware anything.** The repository check is one warning about where the *key* is. Nothing
  stages, ignores or commits on your behalf.

## Where it lives

| | |
|---|---|
| `crates/oslo-base/src/secrets.rs` | `Store`: paths, seal, unseal, rotate, and the user-store shorthands |
| `crates/oslo-base/src/secrets/conf.rs` | `secrets.conf`: parsed, and edited line-wise |
| `crates/oslo-base/src/secrets/key.rs` | `KeySource`: a file, or a program, and what fences the program |
| `crates/oslo-base/src/secrets/recipient.rs` | who a store encrypts to, and what it makes of one it cannot use |
| `crates/oslo-runtime/src/lua/api/secret.rs` | `oslo.secret`, and what a plugin's handle may reach |
| `crates/oslo-runtime/src/plugin/loading.rs` | which plugin is loading, which is what attribution means here |
| `src/cli/secret.rs` | the command line, with `key`, `recipient` and `run` beside it |
| `vendor/age`, `vendor/age-core` | the format, vendored and cut down |
