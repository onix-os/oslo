# mode: posix
# ...unless the handler exits itself, which wins.
trap 'echo cleanup; exit 9' EXIT
echo body
exit 3
