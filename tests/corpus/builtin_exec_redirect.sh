# mode: posix
exec 3> out.txt
echo written >&3
exec 3>&-
cat out.txt
