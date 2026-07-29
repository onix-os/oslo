# mode: posix
# A brace group is not a barrier: the failure inside it is the group's failure.
set -e
echo before
{ false; echo NOT_IN_GROUP; }
echo NOT_REACHED
