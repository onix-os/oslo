# mode: posix
# ${x:+word}
x=set
echo "[${x:+yes}]"
unset y
echo "[${y:+yes}]"
z=
echo "[${z:+yes}]"
