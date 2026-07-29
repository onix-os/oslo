# mode: posix
# `set -a` marks every subsequent assignment for export, which is what `set -a; . /etc/os-release`
# relies on. It applies to any assignment, not just `name=value`, so the `for` variable is checked
# too — and `set +a` has to stop it again.
set -a
V=exported
for F in loopvar; do :; done
set +a
W=private
env | grep -E '^(V|F|W)=' | sort
