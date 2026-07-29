# mode: posix
# History expansion is a prompt feature. A non-interactive shell must leave `!` and `^` alone:
# rewriting them here would let text the shell was *handed* — a filename, a here-doc, an argument
# built from data — turn into a different command than the one written.
echo '!!'
echo !!
echo "!!"
echo pre !$ post
echo a!=b
echo ^old^new
b=
echo "a!$b"
echo tail!
echo done
