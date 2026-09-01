# `oslo fmt`

```sh
oslo fmt script.sh            # to standard output
oslo fmt -w src/*.sh          # in place
oslo fmt --check .            # status 1 and a list, changing nothing
cat script.sh | oslo fmt      # a filter
```

fish ships `fish_indent`. No other shell has a formatter worth the name, and the usual reason is
that writing one means writing the language twice: a parser to read the program and a printer to
write it back out, with every construct the printer forgot silently deleted from whatever it was
given.

**oslo's parser already paid for this.** [rune](https://github.com/onix-os/rune)'s tree holds every
byte of the input — whitespace, comments, and the text of anything that did not parse — so
`Tree::reconstruct` is the identity function. A formatter on that tree is not a second
implementation of shell. It is a walk that changes the space *between* tokens and copies everything
else out of the source.

What follows from that is the safety argument, and it is the whole feature: **a construct the walk
has never heard of comes out as the text it was.** There is no shape of program that comes back
changed because nobody anticipated it. The worst that can happen to one is that it comes back
unimproved.

## The two invariants

* **Idempotence** — `format(format(x)) == format(x)`.
* **Meaning is preserved** — `parse(format(x))` has the same *node* tree as `parse(x)`, and the same
  sequence of significant token texts. Only trivia and separators may differ.

Both are asserted over the 432 scripts in `tests/corpus/` — the same corpus the POSIX differential
suite runs — rather than over samples written to make a formatter look good. 423 of them format;
the other nine are the ones that do not parse, eight of which are fixtures written to be broken.

## What it will not touch

The text of a word, a quoted string, a comment, an expansion, a `[[ ]]` test, or a **here-document
body**. One deliberate exception, below: a block of [argc declarations](argc.md) is lined up, because
it is a declaration rather than prose. The last is the sharpest case: those bytes are data, they have to begin in column zero, and
a formatter that indented them has changed what the program does. They are copied through
untouched, at whatever indentation surrounds them:

```sh
if a; then
    cat <<E
  body
E
fi
```

## What it decides

Indentation, the spaces between words, where `then` and `do` sit, runs of blank lines, and trailing
whitespace. Four spaces by default; `--indent N` or `--tabs` says otherwise.

Some of it is not the formatter's to decide at all, and those are the interesting ones:

**A line break somebody wrote is a line break that stays.** A chain of four commands joined by `&&`
is a paragraph, and whether it was written on one line or four is a decision about reading it. What
the formatter settles is the indentation of the continuation, which nothing but a formatter ever
gets right by hand:

```sh
a && b ||
    c
long \
    --flag one
```

**One line or several, as it was written** — for a `case` arm and a `{ ...; }` group. A table of
short arms is a table, and expanding every one of them to three lines turns something readable into
something to scroll past. The one exception is a bracket that cannot keep its promise: a `while`
inside `( … )` puts its body on lines of its own whatever the bracket wanted, so the bracket opens
out too rather than disagreeing with itself the second time it is formatted.

**A comment stays on the line it was written on.** `echo a # why` says something about that command;
a comment on a line of its own says something about what follows. Moving one to where the other goes
changes what it appears to be about, which is the one thing a formatter must not do to prose.

**A redirection is written as one piece.** `2>&1`, `>file`, `<<EOF` — because the space is where the
meaning goes: `2 >&1` is a command taking `2` as an argument and sending its output to fd 1, which is
not what `2>&1` says. The one place a space goes back in is before a target that would weld itself to
the operator, so `> >(tee log)` does not become `>>(tee log)`.

**Brackets keep their distance.** `( ( echo x ) )` is not tidied to `((echo x))`, because `((` opens
an arithmetic command and the program would stop meaning what it said.

## argc declarations, lined up

Behind the **`argc`** cargo feature, and on by default where that is built in: a build that cannot
*run* a declaration does not tidy one either.

```sh
# @describe Deploy a thing
# @flag -n --dry-run say what would happen
# @option -t --tries <N> how many times
# @option --verbose noisier
# @arg target! where to
# @env TOKEN! the credential
```

becomes

```sh
# @describe Deploy a thing
# @flag     -n --dry-run   say what would happen
# @option   -t --tries <N> how many times
# @option      --verbose   noisier
# @arg      target!        where to
# @env      TOKEN!         the credential
```

**This is the one place the formatter rewrites a comment, and the reason is that an argc block is
not prose.** It is a declaration of what a script takes, in the one place a shell will let a
declaration live, and it is read by a parser rather than by a person. Lining up a table is what a
formatter is for; leaving this one crooked because of where the language chose to put it would be
honouring the letter of the rule against its point.

**Padding cannot change what argc reads.** `parse_tail` is `preceded(space1, rest.trim())` — every
run of whitespace between two tokens is already discarded and every description already trimmed, so
this pass splits on whitespace and joins with whitespace and never has to know what any of the
fields *mean*. The tests check the token sequence is identical before and after, not just that the
output looks right.

Four columns: the tag, the short flag, the long spellings and their notation, the description. A
name — `target!`, `TOKEN!` — is not a flag, so it sits where a short flag sits and never widens the
column the `--long` spellings share. A `@describe` or `@cmd` puts its text in the second column,
because that text *is* the whole of what the tag says.

A **block** is a maximal run of tag lines at column zero — column zero because that is argc's own
rule, so an indented `# @option` is a comment and lining it up would dress it as something it is
not. A plain comment ends a block, because `@describe` and `@cmd` continue onto the comment lines
under them and that text belongs to the tag above it.

An unknown tag is laid out as text rather than guessed at, and a description beginning with `<` or
`-` may be taken for part of the signature and pushed one column left. Both are cosmetic: the words
and their order never change, so the worst case is a line that is not improved.

## What it refuses

A script that does not parse. There is no tree worth reformatting under a missing `fi`, and the
output would be a second mistake laid over the first — so the file is left exactly as it is and
every error is reported with its line and column:

```
oslo fmt: deploy.sh: not formatted, because it does not parse
  deploy.sh:12:1: this `if` was never closed
```

Every error rather than the first, because a script with four mistakes in it is four things to fix
and reporting one at a time is how a formatter becomes a thing people run four times.

## Where it lives

The engine is **rune**'s — its own `format` module — and the command is **oslo**'s
(`src/cli/fmt.rs`, with the argc pass beside it) — the same
split as parsing and lowering. rune owns the tree and the guarantees about it; oslo owns the verb a
person types. Anything else that wants to format shell gets the engine without taking the shell with
it.

**Core, not a cargo feature.** rune is linked whatever else is turned on, the tree is there whether
or not anyone formats, there is no new dependency, and a `#[cfg]` seam would buy almost no bytes.
