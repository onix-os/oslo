# mode: posix
# R7.5: `wait -n` with no operands consumes, so a loop over it drains and then reports 127.
# A child an operand-bearing `wait` already claimed is not handed back by a later `-n` either,
# which is what keeps a fan-in loop from returning the same job forever.
sh -c 'exit 3' &
a=$!
sh -c 'exit 4' &
b=$!
wait "$a"
echo "a=$?"
wait "$b"
echo "b=$?"
wait -n 2>/dev/null
echo "drained=$?"
