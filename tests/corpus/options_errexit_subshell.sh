# mode: posix
# A subshell that fails fails the parent too.
set -e
echo before
(false)
echo NOT_REACHED
