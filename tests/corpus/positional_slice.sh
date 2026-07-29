# mode: bash
# `${@:n}` and `${@#pat}` — the positional twin of the whole-array operators.
#
# The list being sliced is `$0 $1 $2 …`, so `${@:1}` is the first argument. `$0` itself is not
# printed anywhere here: it is the shell's own name and differs between the two shells.
#
# The regression this guards is the argument-forwarding idiom. `"${@:2}"` used to join the
# positionals with a space and cut *characters* out of that string, so a wrapper script that
# forwarded `"${@:2}"` handed its callee one mangled argument instead of the ones it was given.
set -- one two three
printf '[%s]' "${@:1}"; echo
printf '[%s]' "${@:2}"; echo
printf '[%s]' "${@:2:1}"; echo
printf '[%s]' "${@: -2}"; echo
printf '[%s]' "${@:9}"; echo
printf '[%s]' "${*:2}"; echo
echo "${@:2}"
echo "${*:2}"
# An argument containing the separator is one field before and after the slice.
set -- "a b" c d
printf '[%s]' "${@:1}"; echo
printf '[%s]' "${@:1:1}"; echo
printf '[%s]' "${*:1:2}"; echo
# Unquoted, the slice is still a list — and then gets field-split like any other expansion.
printf '[%s]' ${@:1}; echo
# The pattern operators map over the arguments and keep the count.
set -- a.c b.c c.c
printf '[%s]' "${@#a}"; echo
printf '[%s]' "${@%.c}"; echo
printf '[%s]' "${@^^}"; echo
printf '[%s]' "${@/./-}"; echo
printf '[%s]' "${*%.c}"; echo
# `${#@}` is still the count, not the width of anything.
echo "${#@}"
# Forwarding through a function is the idiom the whole case exists for.
show() { printf '<%s>' "$@"; echo; }
fwd() { show "${@:2}"; }
fwd keep "a b" c
fwd only
# Nothing selected is no argument at all.
set -- x y
set -- "${@:9}"
printf 'count=%s\n' "$#"
