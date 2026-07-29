# mode: posix
# `eval` and `return` are judged like anything else, and exempted like anything else.
set -e
eval "false" || echo "eval rescued"
if eval "false"; then echo NO; fi
f() { return 4; }
f || echo "return rescued with $?"
echo before
f
echo NOT_REACHED
