# mode: posix
printf 'x\ny\n' | while read -r l; do echo "got:$l"; done
echo done
