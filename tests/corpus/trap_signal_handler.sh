# mode: posix
# A trapped signal runs its handler at the next command boundary, not inside the handler.
trap 'echo caught' INT
kill -INT $$
echo after
