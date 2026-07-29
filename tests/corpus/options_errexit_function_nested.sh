# mode: posix
# Two levels deep: the exemption reaches the innermost failing command, not just the outer call.
set -e
g() {
  false
  echo in_g
}
f() {
  g
  echo in_f
}
f || echo rescued
echo survived
