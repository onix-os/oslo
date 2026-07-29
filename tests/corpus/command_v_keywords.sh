# mode: posix
# POSIX lists reserved words among what `command -v` reports, and `type` already agreed. Only
# `command` disagreed, so `command -v if` failed — which modernish treats as a fatal shell bug and
# refuses to initialise on at all.
for w in if then else elif fi for while until do done case esac in function; do
    printf '%s -> [%s] status=%s\n' "$w" "$(command -v "$w")" "$?"
done
command -V if
command -V while

# The kinds that were already reported keep their spelling and their order.
echo "builtin=[$(command -v echo)]"
f() { :; }
echo "function=[$(command -v f)]"
echo "missing=[$(command -v definitely-not-a-command-xyzzy)] status=$?"

# `type` and `command -V` must not disagree about what a name is.
type if
type echo
