# mode: posix
# A loop *body* is judged, unlike the condition.
set -e
for i in 1 2 3; do
  echo "iteration $i"
  false
  echo NOT_REACHED
done
echo NOT_REACHED_EITHER
