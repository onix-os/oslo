# mode: posix
{ echo to_out; echo to_err >&2; } 2>&1 | sort
