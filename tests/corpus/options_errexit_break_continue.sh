# mode: posix
# The idiom `cmd || break` works because the left operand is exempt.
set -e
for i in 1 2 3; do
  [ "$i" -lt 3 ] || break
  echo "kept $i"
done
n=0
while [ "$n" -lt 5 ]; do
  n=$((n + 1))
  [ "$n" -ne 2 ] || continue
  echo "n=$n"
done
echo survived
