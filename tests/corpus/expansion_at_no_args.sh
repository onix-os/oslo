# mode: posix
# `cmd "$@"` with nothing set must run `cmd`, not `cmd ""`.
set --
count() { echo "n=$#"; }
count "$@"
count $@
count "$*"
count ""
