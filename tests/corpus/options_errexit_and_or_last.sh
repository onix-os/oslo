# mode: posix
# The last command of an and-or list is the one errexit judges.
set -e
echo before
true && false
echo NOT_REACHED
