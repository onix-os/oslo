# mode: posix
# A subshell is a copy of the shell: it keeps functions, aliases and positionals.
f() { echo function_visible; }
set -- p1 p2
(f; echo "args=$#"; echo "first=$1")
