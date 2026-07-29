# mode: posix
# With pipefail the pipeline reports the failing stage, and errexit sees it.
set -e
set -o pipefail
echo before
false | true
echo NOT_REACHED
