# mode: posix
# A pipeline under `!` is exempt whichever way it comes out.
set -e
! false
echo after_bang_false
! true
echo after_bang_true
! false | true
echo survived
