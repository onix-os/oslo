# mode: posix
# `$?` is a property of the last command, not of the last list. Every member of an and-or list
# publishes its status before the next member is expanded.
true && false || echo "a=$?"
false || true && echo "b=$?"
sh -c 'exit 4' || echo "c=$?"
sh -c 'exit 4' && echo unreachable
echo "d=$?"
false && echo unreachable
echo "e=$?"
true || echo unreachable
echo "f=$?"
# Starting a job is always successful, whatever ran before it.
false
sleep 0 & echo "bg=$?"
wait
