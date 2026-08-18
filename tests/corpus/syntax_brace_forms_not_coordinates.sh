# mode: bash
# oslo reads `{0:0}` as a stream coordinate. `{4}` and `{1..3}` parse as one too — line 4, lines 1
# through 3 — and they are also a regex repeat count and a brace sequence. This case pins the side
# of that collision bash can speak to: wherever bash leaves a brace alone, oslo must leave it alone.
#
# The rule is that a coordinate goes where a brace expands. Brace expansion runs on a word's source
# text before the lexer, so a command word has already become its several words; whatever still
# holds a literal brace is somewhere bash refused to expand one.

# A scalar right-hand side is text, so the braces survive verbatim.
w=x{1..3}; echo "$w"
w={4}; echo "$w"
w={0:0}; echo "$w"

# An array literal is a word list, so it expands.
a=(x{1,2} {3..4}); printf '%s\n' "${a[@]}"

# A regex owns `{}`. The failure this guards was silent and said yes: the quantifier resolved
# against nothing, `^[0-9]{4}` became `^[0-9]`, and two digits matched a four-digit pattern.
[[ 20 =~ ^[0-9]{4} ]] && echo short-matched || echo short-refused
[[ 2024 =~ ^[0-9]{4} ]] && echo exact-matched || echo exact-refused
[[ a =~ ^a{3} ]] && echo one-matched || echo one-refused
[[ aaa =~ ^a{3} ]] && echo three-matched || echo three-refused
d=2024-05
[[ $d =~ ^([0-9]{4})-([0-9]{2}) ]] && echo "y=${BASH_REMATCH[1]} m=${BASH_REMATCH[2]}"

# A command word still expands, because that ran long before any of this.
#
# `{5}` is deliberately not here. bash leaves a one-item group alone, so it is still a literal brace
# when the tree is walked, and oslo reads it as line 5 — the one place this feature departs from
# bash on purpose rather than by accident. `coordinate_tests` records it, where there is no oracle
# to disagree with.
echo {1..3} {a,b}
