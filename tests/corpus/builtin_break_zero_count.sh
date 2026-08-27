# mode: bash
# `break 0` names no loop at all. bash words it apart from a non-number and leaves the whole nest.
for i in 1 2; do for j in a b; do break 0; echo inner; done; echo outer; done
echo after
