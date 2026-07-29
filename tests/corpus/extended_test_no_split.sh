# mode: bash
# `[[ ]]` is syntax, not a command: its operands are neither field-split nor pathname-expanded,
# and an empty one is still an operand. Lowering it onto an ordinary builtin call lost all three —
# a value with a space became two operands, a `*` was globbed against the working directory, and
# an empty value vanished and shifted the operator into the operand slot.
x="a b"
[[ $x == "a b" ]] && echo ok-space || echo bad-space
[[ -n $x ]] && echo ok-nonempty || echo bad-nonempty

x=""
[[ -n $x ]] && echo bad-empty || echo ok-empty
[[ -z $x ]] && echo ok-zero || echo bad-zero
[[ $x == "" ]] && echo ok-empty-compare || echo bad-empty-compare

# In the working directory of the test there are files; an operand holding `*` must not become
# their names.
touch aaa bbb
x="*"
[[ $x == "*" ]] && echo ok-star || echo bad-star

# The right-hand side of `==` is still a pattern, even when it arrives from a variable — that is
# the one place `[[ ]]` differs from `[ ]`, and it must survive the no-splitting rule.
x=abc
[[ $x == a* ]] && echo ok-pattern || echo bad-pattern
p="a*"
[[ $x == $p ]] && echo ok-var-pattern || echo bad-var-pattern
[[ $x == "$p" ]] && echo bad-quoted-pattern || echo ok-quoted-pattern

# Positional parameters go through the same path.
set -- a "b c"
[[ $2 == "b c" ]] && echo ok-positional || echo bad-positional

# An ERE full of metacharacters is one operand too.
v=42
[[ $v =~ ^[0-9]+$ ]] && echo ok-regex || echo bad-regex
