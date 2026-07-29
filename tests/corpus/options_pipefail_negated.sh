# mode: bash
# `!` is applied to whatever pipefail decided the pipeline's status was.
set -o pipefail
! false | true; echo "$?"
! true | true; echo "$?"
set +o pipefail
! false | true; echo "$?"
