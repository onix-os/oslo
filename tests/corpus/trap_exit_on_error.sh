# mode: posix
# A run that goes wrong is the run cleanup matters most for.
trap 'echo cleanup' EXIT
echo before
false
exit 1
