# mode: posix
x=value
echo "[${x:-gone}]"
unset x
echo "[${x:-gone}]"
f() { echo fn; }
f
unset -f f
f 2>/dev/null
echo "$?"
