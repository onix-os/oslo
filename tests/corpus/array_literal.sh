# mode: bash
# An array literal is elements, not the source text it was written as.
a=(1 2 3)
echo "[$a]"
echo "${a[@]}"
echo "${a[1]}"
echo "${#a[@]}"
echo "${!a[@]}"
echo "${a[*]}"
# A scalar assignment to an array name replaces element 0 only.
a=4
echo "${a[@]}"
# An empty literal is an array with nothing in it, not an unset name.
e=()
echo "count=${#e[@]}"
# Elements are words in list context: an unquoted expansion splits, a quoted one does not.
l='p q'
u=($l)
q=("$l" r)
echo "${#u[@]} ${#q[@]}"
# A plain scalar is a one-element array, which is the same identity that makes $a and ${a[0]}
# the same reference in the other direction.
v=solo
echo "${v[@]} ${#v[@]} ${!v[@]}"
# An unset name is the empty array, not an error.
unset nothing
echo "n=${#nothing[@]} [${nothing[@]}]"
