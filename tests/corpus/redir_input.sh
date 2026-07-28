# mode: posix
printf 'a\nb\n' > in.txt
cat < in.txt
wc -l < in.txt | tr -d ' '
