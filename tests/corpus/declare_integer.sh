# mode: bash
# `-i` makes every assignment to the name arithmetic. The letter was refused outright by `declare`
# and accepted-then-ignored by `local`, so an integer variable held the expression as text and
# `count=count+1` grew a string instead of counting.
declare -i m=2+3
echo "declared=$m"
m=4*5
echo "reassigned=$m"
declare -i c=0
c=c+1
c=c+1
echo "counted=$c"
declare -i bad=notanumber
echo "unevaluable=[$bad]"
declare -i q=6
q+=4
echo "appended=$q"
declare -i w=6
w+=2*3
echo "appended_expr=$w"
declare -i z
z+=7
echo "from_unset=$z"

# `+i` takes it back off.
declare -i x=7
declare +i x
x=2+3
echo "dropped=[$x]"

# A name without the attribute is untouched, and `+=` still concatenates there.
plain=2+3
echo "plain=[$plain]"
p2=6
p2+=4
echo "plain_append=[$p2]"

inner() {
  local -i n=2+3
  echo "local=$n"
  local -i k
  k=4*5
  echo "local_later=$k"
  local -i j=1
  j+=9
  echo "local_append=$j"
  local s=2+3
  echo "local_plain=[$s]"
}
inner
echo "after=[${n-unset}]"
