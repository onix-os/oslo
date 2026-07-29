# mode: posix
# An exit code survives every construct that reports one: it is not collapsed to 1, and it is
# not truncated. `exit` unwinds as an error carrying its code, so every place that turns an
# error into a status has to look at the code rather than assume failure.
for n in 0 1 2 3 4 5; do
  ( exit "$n" ); echo "sub$n=$?"
  echo x | exit "$n"; echo "pipe$n=$?"
  ( exit "$n" ) | cat; echo "notlast$n=$?"
  cat /dev/null | ( exit "$n" ); echo "last$n=$?"
  ! ( exit "$n" ); echo "neg$n=$?"
done
( exit 255 ); echo "max=$?"
