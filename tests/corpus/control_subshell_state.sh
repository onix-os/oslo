# mode: posix
# A subshell is this shell in another process: it keeps functions, positionals, $?, $$ and $0,
# and it exports nothing the parent had not exported.
parent_pid=$$
parent_name=$0
secret=classified
export shown=public
greet() { echo "greet $1"; }
set -- alpha beta
false
(
  echo "status=$?"
  greet "$1"
  echo "count=$#"
  if [ "$$" = "$parent_pid" ]; then echo pid=same; else echo pid=changed; fi
  if [ "$0" = "$parent_name" ]; then echo name=same; else echo name=changed; fi
  echo "leak=$(env | grep -c '^secret=')"
  echo "exported=$(env | grep -c '^shown=')"
)
echo "after=$?"
