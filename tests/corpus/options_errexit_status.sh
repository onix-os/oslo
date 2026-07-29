# mode: posix
# The shell exits with the failing command's own status, not a flat 1.
set -e
echo before
(exit 42)
echo NOT_REACHED
