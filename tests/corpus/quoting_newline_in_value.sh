# mode: posix
v="a
b"
printf '%s' "$v" | tr '\n' '|'
echo
