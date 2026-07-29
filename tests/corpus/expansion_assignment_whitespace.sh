# mode: posix
# Internal whitespace and newlines from an unquoted expansion survive assignment intact.
v="  lead and  trail  "
w=$v
echo "[$w]"
n=$(printf 'x\ny\nz')
printf '%s' "$n" | tr '\n' '|'
echo
echo "[$( echo $n )]"
