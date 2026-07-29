# mode: bash
# `[[ =~ ]]` is the input-validation operator. When it was always false, every guard below took
# the wrong branch while still exiting with a legitimate-looking status.
check() {
    if [[ $1 =~ ^[0-9]+$ ]]; then
        echo "$1: numeric"
    else
        echo "$1: not numeric"
    fi
}
check 42
check 0
check -1
check 4x
check abc
check 12.5

# Anchoring is the caller's job: the match is a search.
[[ abc =~ b ]]; echo "search=$?"
[[ abc =~ ^b ]]; echo "anchored=$?"
[[ abc =~ ^abc$ ]]; echo "whole=$?"

# ERE metacharacters, not globs. `a*` is "zero or more a", so it matches anything.
[[ xyz =~ a* ]]; echo "star=$?"
[[ abc =~ a.c ]]; echo "dot=$?"
[[ cat =~ ^(cat|dog)$ ]]; echo "alt=$?"
[[ cow =~ ^(cat|dog)$ ]]; echo "alt2=$?"
[[ a1 =~ ^[[:alpha:]][[:digit:]]$ ]]; echo "class=$?"

# The pattern usually arrives in a variable.
re='^[a-z]+@[a-z]+\.[a-z]{2,}$'
for addr in user@example.com nope@ plain; do
    [[ $addr =~ $re ]] && echo "$addr ok" || echo "$addr bad"
done
