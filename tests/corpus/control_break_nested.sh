# mode: posix
for i in 1 2; do
    for j in a b; do
        [ "$j" = b ] && break 2
        echo "$i$j"
    done
done
echo after
