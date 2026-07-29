# mode: posix
# `!` negates a whole pipeline and is a reserved word only at its head. Accepting it anywhere else
# would turn `echo a | ! grep -q a` into a search for a command literally named `!` — a plausible
# 127 in place of the syntax error the line actually is.
echo a | ! grep -q a
echo NOT_REACHED
