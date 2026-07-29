# mode: posix
set -e
case "$-" in *e*) echo has_e ;; *) echo no_e ;; esac
set +e
case "$-" in *e*) echo still_e ;; *) echo gone_e ;; esac
set -o pipefail
case "$-" in *pipefail*) echo names_leak ;; *) echo letters_only ;; esac
