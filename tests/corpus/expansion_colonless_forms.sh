# mode: posix
# The colon-less forms test only for unset, not for null.
x=
echo "[${x-def}]"
echo "[${x:-def}]"
echo "[${x+set}]"
echo "[${x:+set}]"
unset u
echo "[${u-def}]"
echo "[${u+set}]"
