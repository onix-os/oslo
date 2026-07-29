# mode: posix
set -o nounset
set -o | grep '^nounset'
set +o | grep 'o nounset$'
set +o nounset
set +o | grep 'o nounset$'
