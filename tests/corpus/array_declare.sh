# mode: bash
# `declare -a` makes an array even with no value.
declare -a q
echo "count=${#q[@]}"
declare -a r=(1 2)
echo "${r[@]}"
declare -p r
# The attribute is inferred from a literal, with or without -a.
declare s=(3 4)
echo "${s[@]} $s"
# A declaration inside a function is local to it.
f() {
  local t=(a b)
  echo "in=${t[@]}"
}
f
echo "out=[${t[@]}]"
