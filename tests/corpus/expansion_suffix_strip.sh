# mode: posix
# ${p%pat} / ${p%%pat}
f=archive.tar.gz
echo "${f%.*}"
echo "${f%%.*}"
p=/usr/local/lib/libfoo.so
echo "${p%/*}"
echo "${p%%/*}[end]"
v=abcabc
echo "${v%abc}"
echo "${v%%abc}"
