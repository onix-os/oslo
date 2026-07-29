# mode: bash
# `name[i]=v` writes an element, not a variable whose name contains brackets.
a=(1 2 3)
a[5]=x
echo "${#a[@]} | ${!a[@]} | ${a[@]}"
# A scalar becomes element 0 when the name grows a subscript.
b=hello
b[2]=world
echo "${b[@]} | ${!b[@]}"
# The subscript is arithmetic, so an unset name in it is 0 and expressions work.
m[x]=1
echo "${m[x]} | ${!m[@]}"
i=1
h=(a b c)
echo "${h[i]} ${h[i+1]}"
# A negative subscript counts back from the end.
echo "${h[-1]}"
# Reading past the end is empty, not an error.
echo "[${h[9]}]"
# The length operator measures the element it selects.
echo "${#h[1]}"
# Unsetting an element leaves a hole; the indices after it keep their numbers.
u=(1 2 3)
unset 'u[1]'
echo "${!u[@]} | ${u[@]} | ${#u[@]}"
