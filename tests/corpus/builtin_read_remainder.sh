# mode: posix
printf 'a::b\n' > i1
IFS=: read x y < i1
echo "two=[$x][$y]"
IFS=: read p q r < i1
echo "three=[$p][$q][$r]"
printf 'a::\n' > i2
IFS=: read s < i2
echo "trailing-pair=[$s]"
printf 'a:\n' > i3
IFS=: read t < i3
echo "trailing-one=[$t]"
printf '1  2   3   \n' > i4
read m n < i4
echo "verbatim=[$m][$n]"
printf '  x\ty  \n' > i5
read u < i5
echo "trimmed=[$u]"
printf ':a:\n' > i6
IFS=: read v w < i6
echo "leading=[$v][$w]"
