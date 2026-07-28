# mode: posix
# ~ expands to $HOME; an unknown user stays literal.
[ "$(echo ~)" = "$HOME" ] && echo home_ok || echo home_bad
echo ~nosuchuser-xyz
echo "~"
