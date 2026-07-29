# mode: bash
# `<(cmd)` runs the command on a pipe and hands its *name* to the caller, so the program opens it
# like a file. This is what `diff <(sort a) <(sort b)` needs, and it was refused by name until a
# sweep of 740 real scripts found that 74 of them — one in ten — could not be parsed without it.
#
# Both directions, plus the cases that decide whether the plumbing is right: two at once (each
# needs its own descriptor), nesting, a redirect target, and a failing body (the *caller's* status
# is what matters, not the substituted command's).
cat <(echo read-form)
echo write-form > >(cat); sleep 0.2
cat <(echo one) middle <(echo two)
cat <(cat <(echo nested))
wc -l < <(printf '1\n2\n3\n')
diff <(printf 'same\n') <(printf 'same\n') >/dev/null; echo "identical=$?"
diff <(echo a) <(echo b) >/dev/null; echo "differing=$?"
cat <(exit 3); echo "caller-status=$?"
x=$(cat <(echo captured)); echo "x=$x"
