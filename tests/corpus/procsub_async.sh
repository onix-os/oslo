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
