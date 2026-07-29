# mode: bash
readonly r=1
unset r 2>/dev/null
echo "unset=$?"
echo "value=$r"
