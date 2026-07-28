# mode: posix
# The argument-forwarding idiom every wrapper script uses.
show() { printf '[%s]\n' "$@"; echo "n=$#"; }
outer() { show "$@"; }
outer "one two" three ""
