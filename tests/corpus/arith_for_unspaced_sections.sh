# mode: bash
#
# `for ((...))` with an empty section written without a space around the separators.
#
# The tokenizer takes the longest match, so an empty condition makes the loop's two section
# separators fuse into one `;;` operator — the same token that terminates a `case` item. Every
# form below therefore has to be checked, not just the infinite loop: the initializer and the
# updater have to survive being split at a `;;` they now sit either side of.
#
# The `case` at the bottom is the regression test for the fix. `;;` is a `case` terminator far
# more often than it is a fused pair of loop separators, so a fix that taught the *tokenizer* to
# split `;;` would break every `case` in every script. The grammar-side fix must leave it alone.

# Empty everything: the idiomatic infinite loop.
n=0
for ((;;)); do
    ((n++))
    ((n >= 3)) && break
done
echo "n=$n"

# Empty condition only, with an initializer and an updater either side of the fused `;;`.
for ((i=0;;i++)); do
    [ "$i" -ge 2 ] && break
done
echo "i=$i"

# Empty initializer and updater, condition present: `;i` and `;)` do not fuse, so this form
# already worked. It is here so that a fix scoped to the fused case cannot quietly break it.
j=0
for ((;j<3;)); do
    j=$((j + 1))
done
echo "j=$j"

# The spaced spelling of the first loop. Same loop, no fused token.
k=0
for (( ; ; )); do
    k=$((k + 1))
    (( k >= 2 )) && break
done
echo "k=$k"

# Regression: `;;` still terminates a case item, including one whose body ends in `))`.
m=0
for x in a b c; do
    case $x in
        a) echo A;;
        b) echo B;;
        *) ((m++));;
    esac
done
echo "m=$m"
