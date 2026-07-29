# mode: posix
# `$LINENO` is the line of the command being run. Every `die() { echo "$0:$LINENO: $*"; }` helper
# in every install script depends on it, and unset it silently reported nothing.
#
# The nested cases are the ones worth asserting: a function body, a loop body and an `if` branch
# all report the line they are written on rather than the line of the construct that entered them.
echo "top=$LINENO"

f() {
  echo "func=$LINENO"
}
f
for i in 1 2; do
  echo "loop=$LINENO"
done
if true; then
  echo "if=$LINENO"
fi
