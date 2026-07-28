# mode: posix
set -- a b c
echo "$*"
IFS=-
echo "$*"
echo "$@"
