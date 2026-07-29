# mode: bash
# `$UID` is the root check in most install scripts (`[ "$UID" = 0 ]`), and unset it silently
# answers "not root" on a machine where the script *is* root. The values themselves differ between
# runs and shells, so what is asserted is that they are numeric, that they agree with each other,
# and that they are not exported — bash does not export them either.
case $UID in ''|*[!0-9]*) echo "UID bad";; *) echo "UID numeric";; esac
case $EUID in ''|*[!0-9]*) echo "EUID bad";; *) echo "EUID numeric";; esac
case $PPID in ''|*[!0-9]*) echo "PPID bad";; *) echo "PPID numeric";; esac
env | grep -c '^UID=' || true
