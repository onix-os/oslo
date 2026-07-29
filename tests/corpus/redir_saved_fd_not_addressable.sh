# mode: posix
# The copy the shell keeps of a redirected descriptor is its own business. It must not land in
# the 3..9 range a script addresses by number, and it must not survive exec into a child, or
# `2>&3` inside a redirected group succeeds against a descriptor nobody opened.
{
  echo hi
  sh -c "echo x 2>&3" 2>/dev/null
  echo "inner=$?"
} > out.txt
cat out.txt
