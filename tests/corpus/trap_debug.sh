# mode: bash
# `trap ... DEBUG` fires before each simple command.
#
# With `$PROMPT_COMMAND` this is bash's preexec/precmd pair, and it is what a prompt integration
# uses to time a command: this one starts the clock, that one draws the elapsed time.
#
# **`$BASH_COMMAND` is not set by oslo and is not tested here.** bash names the command about to
# run in it, which means rendering the parsed command back to shell text — 234 lines of renderer
# for one variable, which was removed as not worth its weight. A hook that needs to know *that* a
# command is starting works; one that needs to know *which* does not. Every case below is written
# to hold under both shells, so what it pins is the firing itself.

count=0
trap 'count=$((count + 1))' DEBUG

# An assignment is a command of its own and fires on its own.
target=world
echo "one command later: $count"

echo hello $target
echo "and another: $count"

# The condition of an `if`, then its body: two commands, two firings.
if true; then :; fi
echo "if is two: $count"

# The handler's own commands must not fire it again. Without that guard the first firing would
# recurse until the stack ran out, so reaching this line at all is the assertion.
trap 'count=$((count + 1)); :' DEBUG
:
echo "no recursion: $count"

# Resetting stops it.
trap - DEBUG
before=$count
:
:
echo "after reset, unchanged: $((count - before))"

# `trap -p` reports it in re-inputtable form, like any other condition.
trap 'echo listed' DEBUG
trap -p DEBUG
trap - DEBUG

# A DEBUG handler runs *between* commands, so it cannot disturb `$?`.
false
trap ':' DEBUG
echo "status survives: $?"
trap - DEBUG
