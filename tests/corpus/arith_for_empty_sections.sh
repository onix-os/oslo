# mode: bash
# An absent condition means "true", so this is the infinite loop, not a no-op.
n=0
for (( ; ; )); do
    ((n++))
    echo "n=$n"
    ((n >= 3)) && break
done
echo "final=$n"
for ((m = 0; ; )); do
    echo "m=$m"
    ((m++))
    ((m > 2)) && break
done
