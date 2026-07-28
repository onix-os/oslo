# mode: posix
set -e
if false; then echo no; fi
echo after_if
false || echo after_or
! false
echo after_bang
while false; do echo no; done
echo done
