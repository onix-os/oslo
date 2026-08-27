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

# A reserved word is an ordinary element inside an array literal. The declaration lexer used to
# return If/Do/In tokens that neither of its two consumers handled, so `declare -a a=(x do y)` was
# refused as a bad array value while a bare `a=(x do y)` accepted it — one shell, two answers.
declare -a reserved=(x do y)
echo "reserved=[${reserved[1]}]"
declare -a words=(if then fi case esac for while until done in)
echo "words=[${words[0]} ${words[4]} ${words[9]}]"
inner() { local -a mine=(do done); echo "local=[${mine[1]}]"; }
inner
