# mode: bash
# A one-command pipeline has nothing to disagree about, with or without the option.
set -o pipefail
false; echo "$?"
(exit 7); echo "$?"
true; echo "$?"
set -o pipefail
echo piped | cat; echo "$?"
