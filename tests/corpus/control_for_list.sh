# mode: posix
for x in a b c; do echo "item:$x"; done
for x in; do echo never; done
echo after
