# mode: posix
mkdir sub
touch sub/a.txt
touch sub/b.txt
d=sub
echo "$d"/*.txt
echo $d/*.txt
