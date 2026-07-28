# mode: posix
# $? must be updated between and-or members, not only per list.
true
false || echo "$?"
false && echo unreachable
echo "$?"
sh -c 'exit 5' || echo "left=$?"
