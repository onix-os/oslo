# mode: posix
# ${x=d} assigns only when x is unset; ${x:=d} also when it is null.
unset a
echo "[${a=one}][$a]"
b=
echo "[${b=two}][$b]"
echo "[${b:=three}][$b]"
# ${x?} and ${x+alt} without the colon likewise test only for unset.
c=
echo "[${c?msg}]"
echo "[${c+alt}]"
echo "[${c:+alt}]"
