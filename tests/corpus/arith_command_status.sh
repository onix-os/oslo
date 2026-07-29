# mode: bash
# The status of `(( expr ))` is inverted with respect to its value.
n=0
if ((n)); then echo "zero is true"; else echo "zero is false"; fi
if ((n + 1)); then echo "one is true"; else echo "one is false"; fi
((3 > 5))
echo "gt=$?"
((5 > 3))
echo "lt=$?"
((-1))
echo "neg=$?"
