# mode: posix
# ${p#pat} / ${p##pat} are anchored shell patterns, not substring searches.
p=/usr/local/lib/libfoo.so
echo "${p##*/}"
echo "${p#*/}"
echo "${p#/usr}"
echo "${p##*.}"
v=abcabc
echo "${v#abc}"
echo "${v##abc}"
