# mode: posix
# A bare `exit` inside the EXIT trap carries out the status the shell was *already* leaving with,
# not the status of the last command the trap itself ran. The trap runs after the result is
# decided, so letting a cleanup step rewrite it would make the script's own status a lie.
#
# `trap 'stty …; exit' 0` is the idiom that shows it: `/usr/bin/bzmore` restores the terminal on
# the way out, and with no terminal to restore the `stty` fails — which turned a successful run
# into exit 1 under oslo while bash and dash both reported 0.
#
# Each case runs in a subshell so the trap and the exit belong to it rather than to this script.

( trap 'false; exit' 0; true ); echo "after-success=$?"
( trap 'false; exit' 0; exit 3 ); echo "after-exit-3=$?"
( trap 'true; exit' 0; exit 5 ); echo "after-exit-5=$?"

# An explicit operand still wins: the trap asked for a status and gets it.
( trap 'exit 7' 0; exit 3 ); echo "explicit=$?"

# A trap that merely fails, without `exit`, does not change the status either.
( trap 'false' 0; true ); echo "no-exit=$?"

# Outside a trap, bare `exit` is still `exit $?`.
( false; exit ); echo "outside=$?"
