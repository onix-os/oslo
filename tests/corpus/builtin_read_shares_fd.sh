# mode: posix
printf 'one\ntwo\nthree\n' > in.txt
{ read -r first; cat; } < in.txt
echo "first=$first"
