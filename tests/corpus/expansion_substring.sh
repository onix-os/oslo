# mode: bash
# ${v:off:len} including negative offsets.
v=abcdefgh
echo "${v:2:3}"
echo "${v:2}"
echo "${v:0:1}"
echo "${v: -3}"
echo "${v: -3:2}"
