# mode: posix
set -e
n=0
while [ "$n" -lt 3 ]; do
  n=$((n + 1))
  echo "body $n"
  [ "$n" -lt 2 ]
  echo "still going"
done
echo NOT_REACHED
