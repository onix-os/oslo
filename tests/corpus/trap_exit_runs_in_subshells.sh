# mode: posix
# A subshell is a shell, so an EXIT trap set inside one runs when it ends. All four fork sites are
# checked because each had its own `process::exit` and fixing one proved nothing about the others:
# an explicit subshell, a command substitution, a pipeline stage, and a background job.
#
# The outer trap must still fire exactly once: `enter_subshell` clears what the parent had, so a
# child only ever runs a handler installed inside itself.
trap 'echo outer' EXIT
(trap 'echo paren' EXIT; echo in-paren)
x=$(trap 'echo subst' EXIT; echo captured)
echo "x=$x"
: | (trap 'echo stage' EXIT; :)
{ trap 'echo background' EXIT; :; } &
wait
echo done
