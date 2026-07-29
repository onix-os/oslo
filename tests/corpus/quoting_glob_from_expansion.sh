# mode: posix
# A metacharacter that arrives via an unquoted expansion still globs; a quoted one never does.
touch g1 g2
p='g*'
echo $p
echo "$p"
q=g
echo "$q"*
echo "$q*"
