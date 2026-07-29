# mode: posix
# `saved=$(trap)` is the save-and-restore idiom, and POSIX carves command substitution out of the
# rule that a subshell resets traps precisely so it works. oslo reset them for listing as well as
# for running, so `saved` came back empty and every such wrapper silently saved nothing.
#
# The output has to be re-runnable, which is why the `SIG` prefix matters: it is what every other
# shell prints and what scripts grep for.
trap 'echo caught' INT
trap 'echo leaving' EXIT
saved=$(trap)
echo "$saved"
trap - INT
echo "after reset:"
trap
echo "restoring:"
eval "$saved"
trap
# An inherited handler is listed but must not run: only the outer EXIT fires, once.
(trap)
