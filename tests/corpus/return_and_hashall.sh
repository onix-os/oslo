# mode: bash
# `return` outside a function and outside a sourced script has no frame to unwind. Raising one
# anyway abandoned the rest of the command list, so `return 5; echo after` printed nothing at all.
return 5
echo "continued=$?"

f() { return 3; }
f
echo "from_function=$?"

# `hashall` is on by default — bash reports `h` in `$-` from the first command — and turning it off
# has to actually stop the hashing rather than being accepted and ignored.
case "$-" in *h*) echo "hashall_on=yes";; *) echo "hashall_on=no";; esac
set +o hashall
case "$-" in *h*) echo "still_on=yes";; *) echo "still_on=no";; esac
hash
hash ls
echo "hash_while_off=$?"
set -o hashall
case "$-" in *h*) echo "back_on=yes";; *) echo "back_on=no";; esac
hash -r
hash
