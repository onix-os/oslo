# mode: posix
printf 'b\na\nc\n' | sort | head -2
echo one two three | tr ' ' '\n' | wc -l | tr -d ' '
