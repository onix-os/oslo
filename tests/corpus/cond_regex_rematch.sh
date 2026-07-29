# mode: bash
# The captures are the reason to use `=~` over a `case` statement: BASH_REMATCH is how a script
# pulls fields out of a line without spawning sed.
line="2024-05-17T09:30:00"
if [[ $line =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}) ]]; then
    echo "whole=${BASH_REMATCH[0]}"
    echo "y=${BASH_REMATCH[1]} m=${BASH_REMATCH[2]} d=${BASH_REMATCH[3]}"
    echo "hh=${BASH_REMATCH[4]} mm=${BASH_REMATCH[5]}"
    echo "count=${#BASH_REMATCH[@]}"
fi

# A group that matched nothing is an empty element, not a missing one, so the numbering of the
# groups after it does not shift.
[[ ab =~ (a)(x)?(b) ]]
echo "opt count=${#BASH_REMATCH[@]} two=[${BASH_REMATCH[2]}] three=[${BASH_REMATCH[3]}]"

# A failed match clears the previous captures rather than leaving them to be misread.
[[ zzz =~ (q)(r) ]]
echo "failed=$? count=${#BASH_REMATCH[@]} one=[${BASH_REMATCH[1]}]"

# $BASH_REMATCH without a subscript is element 0.
[[ hello123 =~ [0-9]+ ]]
echo "bare=$BASH_REMATCH"
