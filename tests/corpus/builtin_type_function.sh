# mode: bash
# `type` on a function prints the function, and reports a function that shadows a builtin as the
# function it is — that is the whole point of asking.
f() { echo hi; }
type f
type -t f
cd() { echo shadowed; }
type -t cd
type -at cd
type -a cd
# The body is reconstructed from the parse tree, so every construct has to survive the trip.
g() {
    for i in 1 2; do echo $i; done
    case $x in a|b) echo m ;; *) : ;; esac
    x=1 y=2 cmd arg >out 2>&1
    while :; do break; done
    echo 'a  b' "q$x" && echo done | cat
}
type g
