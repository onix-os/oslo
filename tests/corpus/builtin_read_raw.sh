# mode: posix
printf 'a\\tb\n' > in.txt
read -r line < in.txt
echo "[$line]"
