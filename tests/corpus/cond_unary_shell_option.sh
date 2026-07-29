# mode: bash
# `-o` asks the live option table, so it cannot disagree with `set -o` about what is in force.
[[ -o errexit ]]; echo "errexit-off=$?"
set -e
[[ -o errexit ]]; echo "errexit-on=$?"
set +e
[[ -o errexit ]]; echo "errexit-off-again=$?"

[ -o nounset ]; echo "posix-nounset-off=$?"
set -u
[ -o nounset ]; echo "posix-nounset-on=$?"
set +u

# An option name this shell does not have is false, not an error — which is what makes
# `[[ -o pipefail ]] || ...` safe to write.
[[ -o no-such-option-here ]]; echo "unknown=$?"

# The two brackets must agree; `[[ -o ... ]]` used to be a syntax error while `[ -o ... ]` worked.
set -f
[ -o noglob ]; echo "posix-noglob=$?"
[[ -o noglob ]]; echo "extended-noglob=$?"
set +f

# `-t` on a descriptor that is not a terminal, and on one that is not a number.
[[ -t 0 ]]; echo "tty-stdin=$?"
[[ -t 99 ]]; echo "tty-99=$?"
