# mode: bash
# "${a[@]}" is one field per element; "${a[*]}" is one field joined by IFS.
a=("x y" z)
printf '<%s>' "${a[@]}"; echo
printf '<%s>' ${a[@]}; echo
printf '<%s>' "${a[*]}"; echo
# The splice rule: the first element joins what precedes, the last joins what follows.
printf '[%s]' pre"${a[@]}"post; echo
# An empty array contributes no field at all, so the neighbours stay one word.
e=()
printf '[%s]' pre"${e[@]}"post; echo
set -- "${e[@]}"
echo "count=$#"
set -- "${a[@]}"
echo "count=$# first=$1 second=$2"
IFS=:
b=(a b c)
echo "${b[*]}"
