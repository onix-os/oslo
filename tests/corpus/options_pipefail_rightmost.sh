# mode: bash
# pipefail reports the rightmost failing stage, not the first one and not the last stage.
set -o pipefail
(exit 3) | (exit 4) | true; echo "$?"
(exit 3) | true | true; echo "$?"
true | (exit 5) | true; echo "$?"
true | true | true; echo "$?"
(exit 3) | (exit 4) | (exit 5); echo "$?"
