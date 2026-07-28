# mode: posix
# ${x:-word} with an unset and with a set parameter; the payload is itself a word.
unset v
d=/tmp/fallback
echo "${v:-plain}"
echo "${v:-$d}"
echo "${v:-$(echo sub)}"
v=real
echo "${v:-plain}"
