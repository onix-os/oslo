# mode: posix
# A pipeline stage is a subshell: `while read` in the last stage sees the pipe but its
# assignments die with it, and a `read` that consumes one line leaves the rest for the next
# command in the same stage.
x=outer
echo one two three | while read -r a b c; do echo "$a|$b|$c"; x=inner; done
echo "x=$x"
printf '%s\n' a b c | { read -r first; echo "first=$first"; cat; }
