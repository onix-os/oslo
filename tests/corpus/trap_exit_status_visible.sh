# mode: posix
# The EXIT handler sees the status the shell is ending with, and does not change it.
trap 'echo "trap saw $?"' EXIT
false
