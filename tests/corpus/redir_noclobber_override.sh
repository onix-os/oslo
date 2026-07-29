# mode: posix
set -C
echo first > out.txt
echo second >| out.txt
echo "clobber=$?"
cat out.txt
echo appended >> out.txt
echo "append=$?"
cat out.txt
echo devnull > /dev/null
echo "devnull=$?"
