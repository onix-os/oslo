# mode: posix
trap 'echo one' INT
trap 'echo two' TERM
trap
trap -p INT
trap - INT
trap
