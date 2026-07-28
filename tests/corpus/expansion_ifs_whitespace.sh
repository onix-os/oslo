# mode: posix
# IFS whitespace collapses runs and is stripped at both ends.
v="   a   b   "
set -- $v
echo "$#"
printf '[%s]\n' "$@"
