# mode: posix
# An assignment RHS is neither field-split nor globbed.
x=$(printf 'a\nb\n')
printf '%s' "$x" | tr '\n' '|'
echo
IFS=:
y=p:q
echo "$y"
