# mode: posix
x=1
eval 'y=$x'
echo "$y"
cmd="echo evaluated"
eval "$cmd"
eval 'echo a; echo b'
