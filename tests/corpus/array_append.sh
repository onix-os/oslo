# mode: bash
# `+=` on an array appends elements; on a scalar it concatenates.
a=(1 2)
a+=(3 4)
echo "${a[@]}"
s=x
s+=y
echo "$s"
# Appending goes after the highest index in use, not after the element count.
b=()
b[3]=z
b+=(w)
echo "${!b[@]}"
