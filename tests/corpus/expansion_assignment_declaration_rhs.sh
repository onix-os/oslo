# mode: posix
# export/readonly RHS follows the same no-split, no-glob rule.
touch k.txt
IFS=:
export e=a:b
readonly r=*.txt
echo "$e"
echo "$r"
