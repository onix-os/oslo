# mode: bash
# `shopt` is a namespace of its own: `set -o` cannot reach these, and `-o` is the bridge.
shopt globstar
shopt nullglob
shopt dotglob
shopt -q globstar; echo "q=$?"
shopt -p dotglob; echo "p=$?"

# The one option rush implements both states of.
shopt -s autocd; echo "s=$?"
shopt autocd
shopt -q autocd; echo "qa=$?"
shopt -u autocd; echo "u=$?"
shopt autocd

# A name no shell has is reported, not guessed at.
shopt no_such_option 2>/dev/null; echo "bad=$?"
shopt -s no_such_option 2>/dev/null; echo "badset=$?"

# `-o` reads and writes the `set -o` set instead, which is the only bridge between the two.
shopt -o -q errexit; echo "oq=$?"
shopt -o errexit
shopt -s -o errexit; echo "so=$?"
shopt -o errexit
