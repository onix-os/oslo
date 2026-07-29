# mode: posix
# Without pipefail only the last stage decides, so a failing left-hand stage is invisible.
set -e
false | true
echo after_false_pipe_true
echo done | cat
echo before
true | false
echo NOT_REACHED
