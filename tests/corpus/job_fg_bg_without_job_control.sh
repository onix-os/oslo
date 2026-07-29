# mode: posix
# Every script is a non-interactive shell, and there `fg` and `bg` have no terminal to
# hand over. bash fails with a diagnostic rather than pretending; so must rush.
sleep 1 &
fg
echo "fg=$?"
bg
echo "bg=$?"
wait
