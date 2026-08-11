# Tabs

Named sessions that keep running when the terminal they were opened in goes away. Start a build,
walk away, close the laptop lid, and pick the same shell up somewhere else with the build still
running in it.

One key reaches all of it, and it means the same thing wherever you press it.

> ## This is in `oslo`, not in `oslo-minimal`
>
> Everything on this page is behind the **`tab`** cargo feature, which is off by default. A release
> publishes two binaries per architecture and they differ in exactly this:
>
> | | the key | `oslo.tab` settings |
> |---|---|---|
> | `oslo` | opens the finder | acted on |
> | `oslo-minimal` | unbound — does whatever it otherwise would | **read and ignored** |
>
> ```sh
> make build                  # the full binary, every feature
> make build TYPE=minimal     # the floor: no tabs, no keeper, no runtime directory
> ```
>
> It costs **56 KB** — 6,331,360 bytes without it against 6,388,704 with — and **no dependency at
> all**: it is `nix`, which oslo already links. Startup is unchanged, because nothing runs until the
> key is pressed: 681 µs ± 133 with it against 707 µs ± 149 without, over 300 runs.
>
> The settings are read in both builds on purpose. A config is shared between machines, and making
> it ask `if oslo.tab then` before setting a key would be a question with only one useful answer.

## What the key does

`^\` by default. Press it anywhere.

| where | you press | what happens |
|---|---|---|
| outside a tab | `^\` | the finder: every tab, plus a `new tab <what you typed>` row |
| outside, in the finder | Esc | nothing — back to your prompt, line intact, no tab made |
| outside, in the finder | pick one | attach to it |
| outside, in the finder | type a name, Enter | make it, attach |
| **inside** a tab | `^\` | the same finder, **this tab listed too** |
| inside, in the finder | Esc | leave — back to the shell you pressed the key in, tab still running |
| inside, in the finder | pick the same one | straight back in, nothing changed |
| inside, in the finder | pick another | leave this one running, attach to that |
| inside, in the finder | type a name, Enter | make it and attach; the old tab keeps running |

**Nothing is ever killed by navigating.** A tab ends when the shell inside it exits, which is what
`exit` has always meant. Leaving one is not ending it.

Nothing is auto-named, either: the create row offers only what you typed, so every tab is called
what somebody meant it to be called. `$TAB` holds the current name, for a prompt to show.

```text
         ┌── pick an existing one ──► attach to it
  ^\ ──► finder ── type a name ──────► make it, attach
         └── Esc ───────────────────► outside: nothing. inside: leave it running.
```

## The finder is oslo's own list widget

The same one `ui filter` uses, so it carries the theme, the border and the key handling everything
else in oslo has — arrows, `^n`/`^p`, fuzzy matching as you type.

The `new tab …` row is **a row, not a second prompt**. It appears only when what you have typed
names nothing already in the list, and it sorts last, so Enter still takes a real match by default
and making a new one is the answer you walk to. Offering to create `work` while a tab called `work`
is right there would make Enter ambiguous at the exact moment it matters.

## How it holds a shell

```text
  you press ^\
       │ fork
       ▼
     keeper ── setsid, holds an flock, owns the pty master, serves a socket, writes the log
       │ fork
       ▼
     oslo ── setsid, TIOCSCTTY on the slave, stdin/stdout/stderr on it
```

Two forks and two sessions, and both matter. The keeper `setsid`s so the terminal it came from is no
longer its controlling terminal — closing that terminal sends it no `SIGHUP`, which is the entire
promise. The shell `setsid`s again and claims the pty, so *it* is a session leader on a terminal of
its own, which is what makes `fg`, `bg`, `^C` and `^Z` inside a tab ordinary rather than forwarded.

The keeper never interprets a byte. Input arrives from a client and goes to the pty; output comes off
the pty and goes to the client and to a log. Every decision about what a keystroke *means* belongs to
the pty's line discipline and the shell standing on it.

The shell inside is `exec`d rather than forked on, which costs one config read when a tab is made.
The key is pressed at a prompt, long after oslo has started the threads that warm the `$PATH` index,
and `fork` carries only the calling thread — a child that carried on would hold locks belonging to
threads that do not exist in it.

## Where it lives

`/tmp/oslo-$UID/tab`, mode `0700`, or wherever `oslo.tab.dir` says.

`/tmp` rather than `$XDG_RUNTIME_DIR` because systemd deletes the latter at last logout unless
lingering is enabled, and a tab dying at logout defeats the point. tmux made the same trade for the
same reason and has lived at `/tmp/tmux-$UID` for twenty years.

**`/tmp` is `1777`, so anybody can create `/tmp/oslo-1000` first and wait.** What would land in it is
a socket carrying a terminal's input and output. So the directory is proven before use — a real
directory, our uid, mode `0700`, not a symlink — and proven on the *descriptor*, not the path, so
nothing can be swapped underneath between the check and the open. A directory that fails is refused
and said so at the prompt:

```
oslo: tab: /tmp/oslo-1000/tab is mode 775, and must be 700 — refusing to use it
```

Each tab is four files: a socket, a lock, its metadata and its output log. Liveness is the lock —
asked by trying to take it — so a tab whose keeper was killed is swept by whoever next lists.

## Two backends

```lua
oslo.tab.daemon = false   -- default: a keeper per tab
oslo.tab.daemon = true    -- one registry process, as tab-rs does it
```

Everything on this page is true of both. What changes is who a client talks to:

```text
  daemon = false                        daemon = true

  client ──socket──► keeper             client ──socket──► daemon
                       │ pty                                 │
                       ▼                                     ├─► keeper ─► oslo
                     oslo                                    └─► keeper ─► oslo

  list  = read the directory            list  = ask the daemon
```

**The keeper is the same process in both.** The daemon is a registry and a splice in front of
machinery that already worked — it reads none of what it copies, because the framing between a
client and a keeper is settled elsewhere and a middleman that understood it would be a second place
to keep in step.

There is no service to install. The first client that needs a daemon forks one, exactly as a client
forks a keeper without one, and a daemon nothing has asked for in ten minutes gives up its socket and
exits. A machine with no tabs has no oslo process running either way.

The daemon uses a unix socket rather than tab-rs's localhost TCP port and auth token: on Linux the
filesystem already answers the question the token exists to answer, and a port is reachable by every
process on the machine.

## The key, and why it is spelled twice

```lua
oslo.tab.key = "ctrl-\\"   -- any control chord
```

Whatever this is gets **swallowed from every program running inside a tab** — that is what a proxy
watching for a key means. It rules out `^X`, which is nano's Exit and emacs' prefix. `^\` is the
default because nothing in common use binds it.

Outside a tab the shell owns the terminal, so this is an ordinary editor binding. Inside, the client
owns it and the shell is behind a pty, so the client takes the byte out of the stream before
forwarding. Same key, same finder, two mechanisms.

And the byte is not always a byte. The shell inside a tab asks the terminal for the Kitty keyboard
protocol — `\x1b[>5u`, so its line editor can tell `^I` from Tab — and that request is *output*, so
it travels out through the client to the real terminal. From then on the chord arrives as
`\x1b[92;5u` rather than as `0x1c`:

```text
  legacy     ^\  ──►  1c
  kitty      ^\  ──►  1b 5b 39 32 3b 35 75        "\x1b[92;5u"
```

Both are matched. A client watching for one byte works right up until the first prompt is drawn and
then never fires again, which is as confusing as it sounds because nothing looks broken.

## Settings

```lua
oslo.tab.key = "ctrl-\\"       -- opens the finder, in a tab or out of one
oslo.tab.daemon = false        -- one keeper per tab, rather than a registry process
oslo.tab.dir = ""              -- empty means /tmp/oslo-$UID/tab
oslo.tab.log_bytes = 1048576   -- how much output to keep for the replay on attaching
```

A key name that cannot be one byte is reported against the line that wrote it and the default is
kept — the alternative is a tab with no way out.

## What it does not do

No panes, no splits, no status bar, no layouts, no scrollback of its own. A tab is one shell that
outlives its terminal, and that is the whole idea. Anything that wants two things on one screen
wants a multiplexer, and there are good ones.
