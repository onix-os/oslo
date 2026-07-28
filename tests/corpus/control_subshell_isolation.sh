# mode: posix
x=outer
(x=inner; echo "in=$x")
echo "out=$x"
cd /
(cd /usr) 2>/dev/null
pwd
