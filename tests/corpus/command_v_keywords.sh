# mode: posix
# needs-bash: 5.3
# POSIX lists reserved words among what `command -v` reports, and `type` already agreed. Only
# `command` disagreed, so `command -v if` failed — which modernish treats as a fatal shell bug and
# refuses to initialise on at all.
#
# Gated on 5.3 for the `status=$?` on the `missing=` line below, which is nothing to do with
# `command`: bash 5.3 stopped letting a command substitution update `$?` for the rest of its own
# word *under --posix*, so 5.2 reports 1 there and 5.3 reports 0. oslo follows 5.3. The bash-mode
# half of that behaviour, which both versions agree on, is pinned by
# status_after_command_substitution.sh — which needs no gate.
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
