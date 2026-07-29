# mode: posix
# A function called in a judged position dies at its first failing command.
set -e
f() {
  echo in_f
  false
  echo NOT_REACHED
}
f
echo NOT_REACHED_EITHER
