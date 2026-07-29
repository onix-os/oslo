# mode: bash
n=0
for ((;;)); do
    ((n++))
    ((n >= 3)) && break
done
echo "n=$n"
