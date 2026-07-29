# mode: bash
# `hash -r` is a request to forget, and forgetting prints nothing.
hash -r
hash
hash -r; echo "r=$?"
hash no-such-command-xyz 2>/dev/null; echo "miss=$?"
hash -Z 2>/dev/null; echo "flag=$?"
