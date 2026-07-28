# mode: posix
printf 'default target\n' > in.txt
read < in.txt
echo "[$REPLY]"
