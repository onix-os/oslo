# mode: posix
# A for with no list iterates "$@".
set -- "a b" c
for x; do echo "[$x]"; done
