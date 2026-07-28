# mode: posix
for i in 1 2 3 4 5; do
    [ "$i" = 2 ] && continue
    [ "$i" = 4 ] && break
    echo "i=$i"
done
echo after
