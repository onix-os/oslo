# mode: posix
trap 'echo cleanup' EXIT
echo before
exit 3
echo NOT_REACHED
