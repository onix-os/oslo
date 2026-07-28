# mode: bash
set -o pipefail
false | true; echo "$?"
true | false; echo "$?"
true | true; echo "$?"
set +o pipefail
false | true; echo "$?"
