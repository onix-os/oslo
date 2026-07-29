# mode: bash
# Only the statuses are compared: rush tracks which functions are executing but has no LINENO,
# so the line and source fields it prints are placeholders. The status is what the stack-trace
# idiom `i=0; while caller $i; do i=$((i+1)); done` actually reads.
caller > /dev/null; echo "top=$?"

f() { caller > /dev/null; echo "in=$?"; caller 0 > /dev/null; echo "f0=$?"; caller 3 > /dev/null; echo "f3=$?"; }
f

# `caller 0` names the *calling* function, so it only answers once there is one.
g() { f; caller 1 > /dev/null; echo "g1=$?"; }
g

caller 0 > /dev/null; echo "after=$?"
