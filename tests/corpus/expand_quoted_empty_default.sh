# mode: bash
# A quoted expansion is one field even when what it expands to is empty.
#
# `"${x-}"` is how every `set -u`-safe script spells "this may be unset". The payload has no parts
# at all, so expanding it produced *no fields* and the word disappeared rather than becoming an
# empty one. Two ways that showed up, both silent:
#
#   * in an argument list, an argument was simply dropped;
#   * inside `[[ ]]`, the operator slid left into the operand slot and the test died with
#     `==: unary operator expected` — which is not a wrong answer, it is a parse error at runtime.
#
# Found in `atuin init bash`, whose very first line is `[[ ${__atuin_initialized-} == true ]]`.
# The integration could not get past line 5.
#
# The unquoted case is the other half and is *not* a bug: `${x-}` with nothing in it really is zero
# fields, because splitting an empty result yields nothing. Both directions are checked here, since
# fixing one by breaking the other is the obvious wrong repair.

set -- "${undefined-}"
echo "quoted empty default: $#"

set -- ${undefined-}
echo "unquoted empty default: $#"

set -- "${undefined:-}"
echo "quoted empty colon-default: $#"

set -- "${undefined-fallback}"
echo "quoted non-empty default: $# [$1]"

defined=value
set -- "${defined+}"
echo "quoted empty alternative: $#"

# `"$@"` with no positionals is the one expansion that is legitimately zero fields, and it must
# stay that way — `cmd "$@"` with nothing to forward has to run `cmd`, not `cmd ""`.
set --
set -- "$@"
echo "empty dollar-at: $#"

# The shape it was actually found in.
[[ ${undefined-} == true ]]
echo "compare against unset: $?"

[[ ${undefined-} ]]
echo "test unset alone: $?"

[[ ! ${undefined-} ]]
echo "negated: $?"

# It has to survive being part of a larger word too, where the empty run is concatenated rather
# than standing on its own.
set -- "prefix${undefined-}suffix"
echo "inside a word: $# [$1]"
