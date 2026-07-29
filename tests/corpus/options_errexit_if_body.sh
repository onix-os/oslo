# mode: posix
# then- and else-branches are ordinary command lists, judged like any other.
set -e
if true; then
  echo in_then
  false
  echo NOT_REACHED
fi
echo NOT_REACHED_EITHER
