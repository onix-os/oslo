# mode: posix
trap 'echo should_not_run' EXIT
trap - EXIT
echo body
