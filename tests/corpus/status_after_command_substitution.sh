# mode: bash
# A command substitution is a command that *ran*, so `$?` later in the same word reports it.
# Expansions happen left to right, and each substitution updates the status as it finishes.
#
# oslo recorded that status and never published it: `$?` beside a substitution read whatever the
# previous *command* had left, so `echo "[$(exit 7)] $?"` said 0. Always the wrong number, and
# silently — which is why it survived 400 corpus cases. The one case that touched it,
# command_v_keywords.sh, is `# mode: posix`, where bash 5.3 answers 0 for its own reasons; the
# bash-mode half went untested entirely.
#
# No `needs-bash` gate: 5.2 and 5.3 agree about everything here.

# The shape that was wrong, at three different statuses.
echo "seven=[$(exit 7)] status=$?"
echo "one=[$(false)] status=$?"
echo "zero=[$(true)] status=$?"

# The substitution's status, not the previous command's — which is 0 from the `echo` above.
false
echo "after-false=[$(true)] status=$?"

# Left to right: the *last* substitution in the word wins.
echo "two=[$(exit 3)][$(exit 4)] status=$?"

# A word with no substitution in it leaves `$?` alone, so this still reports the `echo`.
echo "plain status=$?"

# The assignment form is POSIX-defined and was already right; kept so the two cannot drift apart.
v=$(exit 5)
echo "assigned=[$v] status=$?"

# In a command's arguments rather than in a string.
printf '%s\n' "arg=[$(exit 6)] status=$?"

# The status survives into the next command as an ordinary `$?`.
echo "x=[$(exit 9)]" >/dev/null
echo "carried=$?"
