# mode: bash
# Operators applied to a *whole* array. One syntax, two meanings: `:` slices the element list,
# and the pattern operators rewrite every element and keep the count. Neither is a string
# operation on the joined text, which is what makes the `[*]` lines worth printing — they join
# *after* the operator ran, not before.
a=(alpha beta gamma)
echo "${a[@]:1}"
echo "${a[@]:1:1}"
echo "${a[@]: -1}"
echo "${a[@]:9}"
echo "${a[*]:2}"
echo "${a[@]#al}"
echo "${a[@]%a}"
echo "${a[@]^^}"
echo "${a[@]/a/-}"
echo "${a[*]#al}"
printf '[%s]' "${a[@]:1}"; echo
printf '[%s]' "${a[@]#al}"; echo
# An element holding the separator survives slicing as one field; slicing the joined text could
# not tell it from two.
b=("x y" z)
printf '[%s]' "${b[@]:0}"; echo
printf '[%s]' "${b[@]:1}"; echo
# A hole shifts what an offset selects: the slice indexes the elements in use, not the subscripts.
c=(p q r)
unset 'c[1]'
printf '[%s]' "${c[@]:1}"; echo
# A scalar is a one-element array to `[@]`, and stays one under an operator.
s=solo
printf '[%s]' "${s[@]:0}"; echo
printf '[%s]' "${s[@]^^}"; echo
# Nothing selected is no field at all, not one empty field.
set -- "${a[@]:9}"
printf 'count=%s\n' "$#"
