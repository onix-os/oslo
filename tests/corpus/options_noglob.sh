# mode: posix
touch a.txt
set -f
echo *.txt
set +f
echo *.txt
