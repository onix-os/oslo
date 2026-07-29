# mode: bash
# The idiom arrays exist for: forwarding a list of arguments without losing their boundaries.
paths=("a b" "c" "")
show() {
  echo "count=$#"
  for p in "$@"; do echo "arg=[$p]"; done
}
show "${paths[@]}"
# An empty array forwards nothing, so the callee sees no arguments rather than one empty one.
empty=()
show "${empty[@]}"
