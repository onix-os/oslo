# mode: posix
set -C
echo first > out.txt
echo second > out.txt 2>/dev/null
echo "status=$?"
cat out.txt
