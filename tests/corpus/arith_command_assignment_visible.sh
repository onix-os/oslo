# mode: bash
# `(( ))` is not a subshell: every side effect has to survive it.
x=5
((x++))
echo "post=$x"
((++x))
echo "pre=$x"
((x += 10))
echo "compound=$x"
((y = x * 2))
echo "new=$y"
count=0
for word in a b c; do
    ((count++))
done
echo "count=$count"
