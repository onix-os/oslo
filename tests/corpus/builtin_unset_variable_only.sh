# mode: posix
v=1
unset -v v
echo "[${v:-gone}]"
echo "status=$?"
f() { echo fn; }
unset -v f
f
