# mode: bash
# A process substitution's child is asynchronous. The shell must not wait for it: `exec 8< <(gen)`
# copies the read end into descriptor 8, so the generator is *meant* to outlive the `exec`, and
# waiting for it never returns. modernish's LOOP is built exactly that way and hung against oslo.
exec 8< <(yes ok)
echo "reached=$?"
read -r line <&8
echo "read=[$line]"
exec 8<&-

# The ordinary, finite cases still behave.
echo "simple=[$(cat <(echo hi))]"
diff <(printf 'a\n') <(printf 'a\n') >/dev/null && echo "diff=same"

# A generator that outlives one command and is consumed by the next.
exec 7< <(printf 'one\ntwo\nthree\n')
read -r a <&7
read -r b <&7
echo "a=$a b=$b"
exec 7<&-

# The pipe must not sit in the 3..9 range a script is entitled to redirect. `pipe()` hands back the
# lowest free number, which is 3 in a plain shell, so the command's own redirection dup2'd over the
# substitution it was reading and `cat` silently saw an empty file.
echo "guarded3=[$(cat <(echo hi) 3>/dev/null)]"
echo "guarded9=[$(cat <(echo hi) 9>/dev/null)]"
echo "guarded_both=[$(cat <(echo a) <(echo b) 3>/dev/null 4>/dev/null)]"
