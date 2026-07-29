# mode: posix
export A=" spaced "
echo "[$A]"
export B="'quoted'"
echo "[$B]"
sh -c 'echo "child=[$A][$B]"'
