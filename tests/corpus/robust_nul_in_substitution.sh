# mode: posix
x=$(printf 'a\0b')
printf '[%s]\n' "$x"
echo STILL_ALIVE
