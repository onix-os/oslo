# mode: posix
! false
echo "$?"
! true
echo "$?"
if ! false; then echo negated; fi
