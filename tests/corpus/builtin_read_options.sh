# mode: bash
printf 'hello world\nsecond line\n' > in.txt
{ read -n 5 a; echo "n5=[$a]"; read rest; echo "rest=[$rest]"; } < in.txt
{ read -N 4 b; echo "N4=[$b]"; } < in.txt
{ read -d ' ' c; echo "d=[$c]"; } < in.txt
{ read -r -n 3 d; echo "rn=[$d]"; } < in.txt
{ read -rn3 e; echo "cluster=[$e]"; } < in.txt
{ read -p 'PROMPT> ' f; echo "p=[$f]"; } < in.txt
{ read -s g; echo "s=[$g]"; } < in.txt
{ read -u 0 h; echo "u=[$h]"; } < in.txt
{ read -t 0 i; echo "t0=$?"; } < in.txt
{ read -- j; echo "dd=[$j]"; } < in.txt

# -r is the difference between an escape and a byte of data.
printf 'one\\two\nthree\\\n' > esc.txt
{ read -r r1; echo "raw=[$r1]"; } < esc.txt
{ read r2; echo "cooked=[$r2]"; } < esc.txt

# -d '' is the NUL delimiter `find -print0` needs.
printf 'p\0q\0' > nul.txt
{ read -d '' n1; read -d '' n2; echo "nul=[$n1][$n2]"; } < nul.txt

# -u reads a descriptor other than stdin, and leaves stdin where it was.
exec 7< in.txt
read -u 7 u1
echo "u7=[$u1]"
exec 7<&-

# -a binds every field, not just the first: the count and the elements both matter.
printf 'a:b:c\n' > f.txt
{ IFS=: read -a arr; echo "arr=${#arr[@]}:[${arr[0]}][${arr[1]}][${arr[2]}]"; } < f.txt
{ IFS=: read -a arr; echo "all=[${arr[@]}] star=[${arr[*]}]"; } < f.txt
# A shorter line replaces the array; it does not leave the old tail behind.
arr=(1 2 3 4 5)
printf 'x y\n' | { read -a arr; echo "replaced=${#arr[@]}:[${arr[*]}]"; }
# Operand names after -a are ignored, as is field splitting under -N.
printf 'a b c\n' | { read -a arr extra; echo "ignored=${#arr[@]} extra=[${extra}]"; }
printf 'a b c\n' | { read -N 5 -a arr; echo "exact=${#arr[@]}:[${arr[0]}]"; }
# Nothing to read leaves an empty array and a failing status.
printf '' | { read -a arr; echo "eof=$? n=${#arr[@]}"; }
while read -a row; do echo "row=${#row[@]}:[${row[0]}]"; done < in.txt

# A timeout that cannot be waited for is rejected, not rounded down to a probe.
read -t -1 bad < in.txt
echo "neg=$?"
read -t nope bad < in.txt
echo "nan=$?"
