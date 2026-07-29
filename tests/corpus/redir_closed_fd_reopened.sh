# mode: posix
# R11.C1. `open(2)` returns the lowest free descriptor, so in a clean shell `3>` opens the file
# *on fd 3* — the number the script asked for. `dup2(3, 3)` is then a POSIX no-op, and dropping
# the handle afterwards closes the descriptor the redirection had just set up. Nothing about that
# needs `exec`: a plain group redirection reproduces it.
{ echo one >&3; echo two >&3; } 3> out.txt
cat out.txt
# The descriptor must reach a child process too. It does only because it is not close-on-exec,
# which is the reason the fix cannot be "leak the handle and hope".
{ sh -c 'echo from-child >&3'; } 3> child.txt
cat child.txt
# And it must be closed again once the group it belonged to has ended.
{ echo after >&3; } 2>/dev/null || echo closed-again
