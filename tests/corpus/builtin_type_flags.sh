# mode: posix
# The option letters are options, not names to look up. `-t` names the kind, `-p`/`-P` are the
# path forms that stay silent for a non-file, and an unknown letter is a usage error.
type -t if
type -t type
type -t nosuchcommand_xyzzy
echo "t=$?"
type -p if
echo "p=$?"
type -P nosuchcommand_xyzzy
echo "P=$?"
type -z if
echo "opt=$?"
type nosuchcommand_xyzzy
echo "miss=$?"
