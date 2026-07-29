# mode: posix
# The classic three-way swap: fd 3 parks stdout so stdout and stderr can trade places, and only
# the swapped stderr reaches the pipe.
swap() { echo to_out; echo to_err >&2; }
swap 3>&1 1>&2 2>&3 3>&- | sed 's/^/piped:/'
