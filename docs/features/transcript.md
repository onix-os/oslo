# What a finished line leaves behind

Running a command normally leaves its prompt on the screen: the hostname, the branch, the vi mode
and the duration, sitting above the output for as long as the scrollback keeps them. With
`oslo.transcript.rule` set, oslo replaces that prompt with a record of *what was run* — which is the
half anybody rereads, and the half that survives being copied out of a terminal into a bug report.

It is off unless a config asks for it. A setting that changes the shape of the scrollback is not one
to assume.


```lua
oslo.transcript.rule   = "-"      -- empty is off, which is the default
oslo.transcript.prefix = ""       -- optional, inside the brackets
oslo.transcript.style  = "1"      -- the divider's colour; an index, a hex value or a name
```

With a rule set, running a line **replaces its prompt** with what was run:

```

-------------------------------------------------[ cargo test --lib ]---

running 798 tests
```

A blank row sits on each side of the block, so it reads as a divider rather than as another line of
the output above or below it. The one *above* is written before the prompt, not before the block —
the prompt is what the block replaces, and it wants the same air whether or not the line typed at it
turns into one.

**Except after `clear`**, which puts the cursor at row one: a blank written there spends exactly the
space the clear was asked for. `clear` and `reset` are recognised by name, alone or through `tput`.
The alternative is asking the terminal where the cursor is before every prompt — a round trip per
prompt, on a link that may be slow, for one blank line — so a screen blanked some other way keeps
its row, which is a cosmetic miss rather than a broken prompt.

The prompt block is cleared, a rule runs into the command at the right-hand end, and the output
follows. What scrolls back is then a record of *what was run* — which is the half anybody rereads,
and the half that survives being copied out of a terminal into a bug report. A prompt carrying a
hostname, a branch, a vi mode and a duration is none of those things once the moment has passed.

**The mark at the left end is how the command *above* ended.** A frame is drawn between Enter and
the command starting, so it can never report its own status — the shell learns that once the output
has already scrolled past. What it can report is the command before it, and the left end of the rule
is where that is true: it sits directly under the last line of that command's output.

```
------------------------------------[ echo one ]---
one
---[ 0 ]----------------------------[ false ]---
---[ 1 ]----------------------------[ echo two ]---
two
```

The same run of rule leads into it as trails the command, so the row reads as a rule with a bracket
let into each end rather than one that starts at a bracket and ends at another. The first frame of a
session carries nothing, because nothing has run.

**Right-aligned, because that is where the eye already is.** The command sits beside the output it
produced rather than at the far left with a screen of rule between them, and a column of brackets
down the scrollback reads as a list of what was run. Three cells of rule carry on past the bracket
so the line reads as a rule the command interrupts rather than one that stops at it.

`rule` is a **unit repeated to the width** of the terminal, so `"-"` is solid and `"- "` is dashed.
The command between the brackets is left exactly as it was typed. A command too wide for the row
keeps its brackets and loses the lead-in rather than being cut.

**The divider is an indexed colour, not a theme slot.** `"1"` is palette entry 1, which a terminal
can retint on its own without the shell being told — hexe's `OSC 1330` namespaces do exactly that,
so the divider can be recoloured for a whole pane after the fact. A theme slot would be resolved
here and baked into the bytes, and no palette could reach it afterwards. A hex value or a colour
name works too; anything that is not a colour falls back to the theme's `prompt.aside`.

**Every line of a multi-line command gets its own brackets** — a paste, a heredoc:

```
------------------------------------------[ for f in *.rs; do ]---
                                          [ echo "$f" ]
                                          [ done ]
```

A stem would say "this belongs to the thing above", which is what output does; a bracket says "this
is a command", which is what each of these is. The rule is the first row's alone — repeated down the
block it would read as three commands rather than one — and the rows under it hang from where it
stopped.

A line that is only whitespace leaves nothing: there is no command to frame, and a bracket around an
empty one is a worse transcript than none. A key bound with `erase` — see
[the line editor](line-editor.md) — keeps its own ending, since a key that *is* a command was never
meant to be seen, a frame around it least of all.

### Letting another program draw it

```lua
oslo.transcript.command = {
  command    = "pixy",
  args       = { "render", "transcript", "--set", "cmd=$command" },
  timeout_ms = 20,      -- the default
}
```

Whatever it prints goes **between the brackets**, and oslo draws the rule around it.
`$command` is substituted in `args`, the only field there is, since the rest of what a prompt is told
stopped being interesting the moment the command started.

**One line, because that is what such a tool can give.** pixy refuses a control byte in a rendered
string outright, so a contract of "print the whole block" is one it could not meet — and the rule and
the tree rows of a multi-line command are oslo's either way; the split is
where it has to be. Trailing line endings are cut, since a program that prints a line ends it and
oslo is about to end it again.

The prefix and the command stay as the fallback: a renderer that is missing, fails or overruns
leaves the command as it was typed rather than nothing.

**The deadline is short and there is no `async`.** This runs between Enter and the command starting.
A frame that arrived after the output had already begun would not be a frame, and there is nothing
sensible to draw in the meantime — so a tool that overruns is killed and the rule is used.

### The frame marks

A transcript already sits inside `OSC 133`'s region — between `B`, the start of input, and `C`, the
start of output — so a terminal can fold a whole command with `A`…`D` and needs nothing new. What it
cannot do from `OSC 133` alone is tell the *frame* apart from the prompt, which is what folding
everything **except** the header needs. So the block is wrapped:

```
ESC ] 133  ; A                              prompt start
ESC ] 133  ; B                              input start
ESC ] 1440 ; frame ; begin ; aid=<session>
- - - - - - - - - - - - - - - - - - - -
cargo test --lib
- - - - - - - - - - - - - - - - - - - -
ESC ] 1440 ; frame ; end ; aid=<session>
ESC ] 133  ; C                              output start
running 795 tests
ESC ] 133  ; D ; 0                          command end, with status
```

Fold from `frame;end` to `133;D` and the header stays.

**Its own number, not a key inside `OSC 133`.** That vocabulary is shared with every other shell, and
a key oslo invented there is one those shells' terminals have to guess at. 1440 is adjacent to 133
and clear of hexe's 1330, which made the same call for its palette protocol and reserved 133 for
exactly this reason.

The verb comes first — `frame` is the only one today, so a later `fold` or `title` is another verb
rather than another number, and a terminal that does not know a verb ignores the sequence whole,
which is what every terminal already does with an OSC it has never heard of.

Change the number with `oslo.transcript.osc`, or with `$OSLO_TRANSCRIPT_OSC` for a terminal that has
claimed 1440 for something else without editing a config. A number a terminal already acts on — `0`,
`7`, `133`, `1337` and the rest — is refused and the default used instead: claiming one does not add
a mark, it takes away whatever that number did, silently and far from the line that caused it.

Nothing is written at all when marks are off — a script, a pipe, `-c` — which is the same rule that
governs `OSC 133` here.


## Where it lives

| | |
|---|---|
| `crates/oslo-ui/src/transcript.rs` | the renderer slot, the frame marks, the OSC number |
| `crates/oslo-ui/src/edit/screen.rs` | `transcript`, `given`, `framed` — the layout, as pure functions |
| `crates/oslo-ui/src/edit/session/ending.rs` | which of the three endings a finished line gets |
| `crates/oslo-ui/src/settings/misc.rs` | `oslo.transcript` — `rule`, `prefix`, `style`, `osc` |
| `crates/oslo-runtime/src/startup/transcript.rs` | reading `oslo.transcript.command`, and running it |
| `crates/oslo-runtime/src/startup/read.rs` | the blank row before the prompt |

See [the prompt](the-prompt.md) for what the block replaces, [the line editor](line-editor.md) for
`erase` — the other ending a key can ask for — and [the terminal](terminal-integration.md) for the
`OSC 133` marks a transcript sits inside.
