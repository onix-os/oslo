# mode: bash
# `;&` runs the next branch's body without testing its pattern, all the way down the chain.
for subject in a b c d; do
    case $subject in
        a) echo "one" ;&
        b) echo "two" ;&
        c) echo "three" ;;
        d) echo "four" ;;
    esac
    echo "--"
done
