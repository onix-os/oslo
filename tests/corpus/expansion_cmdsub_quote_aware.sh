# mode: posix
# A delimiter inside quotes, or inside a nested expansion, must not end the construct.
echo "$(echo "(")"
echo "$(echo '}')"
unset x
echo "${x:-$(echo "a}b")}"
echo "$(echo "$(echo deep)")"
echo "$( (echo sub) )"
