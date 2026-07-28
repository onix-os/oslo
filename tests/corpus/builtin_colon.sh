# mode: posix
:
echo "$?"
unset x
: ${x:=defaulted}
echo "$x"
i=0
while :; do
    i=$((i + 1))
    [ "$i" -ge 3 ] && break
done
echo "$i"
