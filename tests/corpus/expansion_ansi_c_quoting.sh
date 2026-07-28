# mode: bash
x=$'a\tb'
printf '%s' "$x" | tr '\t' '|'
echo
y=$'line1\nline2'
printf '%s' "$y" | tr '\n' '|'
echo
