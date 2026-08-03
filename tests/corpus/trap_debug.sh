# mode: bash
# `trap ... DEBUG` fires before each simple command, with `$BASH_COMMAND` naming what is next.
#
# This is not a completeness item. It is the hook every shell integration is built on: starship
# times commands through it, hexe's `__shp_preexec` is installed with exactly `trap '...' DEBUG`,
# and bash-preexec — which atuin, and most of the ecosystem, sits on top of — is nothing but a
# DEBUG trap and a `PROMPT_COMMAND`. Without it those tools install cleanly and then do nothing.
#
# `$BASH_COMMAND` is a *re-render* of the parsed command, not the source text. bash does the same,
# and the visible tell is the space it puts after a redirection operator that the script never
# wrote. That normalisation is asserted here on purpose: a hook matching on this text has to see
# what it would see under bash.

trap 'echo "<$BASH_COMMAND>"' DEBUG

# An assignment is a command of its own and fires on its own.
target=world

echo hello $target

# Unexpanded: the trap sees what is about to run, not what it becomes. A hook that wants to veto
# `rm $dir` needs the variable, not the directory it happened to name this time.
echo "$target"

# `2>/dev/null` comes back as `2> /dev/null`, which is bash's spelling and not the script's.
ls /nonexistent 2>/dev/null

# The condition of an `if`, then its body: two commands, two firings.
if true; then echo inside; fi

# The handler's own commands must not fire the trap again. Without that guard this line would
# recurse until the stack ran out.
trap 'echo "<<$BASH_COMMAND>>"; echo nested' DEBUG
echo after

# `trap -p` reports it in re-inputtable form, like any other condition.
trap - DEBUG
echo "reset: $?"

trap 'echo listed' DEBUG
trap -p DEBUG
trap - DEBUG

# A DEBUG handler runs between commands, so it cannot disturb `$?`.
false
trap 'true' DEBUG
echo "status survives: $?"
