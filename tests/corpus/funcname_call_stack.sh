# mode: bash
# `$FUNCNAME` is the call stack a script can read, and it used to be empty — so every log line and
# error handler built on it lost the one thing it carried, silently.
#
# An array: element 0 is the function running now, element 1 is whoever called it. Unset outside
# every function, which is how a script asks whether it is inside one at all.
echo "[${FUNCNAME+set-at-top-level}]"

one() {
    echo "name=$FUNCNAME"
    echo "zero=${FUNCNAME[0]}"
    echo "depth=${#FUNCNAME[@]}"
}
one

outer() { inner; }
inner() {
    echo "chain=${FUNCNAME[0]}/${FUNCNAME[1]}"
    echo "all=${FUNCNAME[@]}"
}
outer

# It empties again on the way out.
echo "[${FUNCNAME+still-set}]"

# Recursion counts every frame.
down() {
    if [ "$1" -gt 0 ]; then
        down $(( $1 - 1 ))
    else
        echo "recursed=${#FUNCNAME[@]}"
    fi
}
down 3
