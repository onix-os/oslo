# mode: posix
# An AND-OR list that short-circuits is exempt from errexit — POSIX exempts "any command of an
# AND-OR list other than the last" — and it stays exempt when the list is the *last* command of a
# compound. The compound merely inherits a status that was never judgeable; inheriting it must not
# make it judgeable.
#
# Found by running every `#!/bin/sh` script on a Debian system under oslo and dash:
# `/usr/sbin/on_ac_power` ends its `if` block with
#   [ "${OFF_LINE_P}" = "yes" ] && [ "${HAS_BATTERY}" = "yes" ] && exit 1
# and oslo exited 1 there while bash, dash and busybox all carried on.
set -e

if true; then false && echo NO; fi
echo after-if

for i in one; do false && echo NO; done
echo after-for

{ false && echo NO; }
echo after-group

while :; do false && echo NO; break; done
echo after-while

# The same shape with `||`, whose last command also never runs.
if true; then true || echo NO; fi
echo after-or

# And a bare failure inside a compound *does* still end the shell, or the exemption would have
# been widened into a hole.
if true; then false; echo NOT-REACHED; fi
echo NOT-REACHED-EITHER
