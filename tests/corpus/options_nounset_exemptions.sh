# mode: posix
# nounset tests for *unset*, not for null, and every defaulting operator is exempt — otherwise
# there would be no way to read a possibly-unset variable while the option is on.
set -u
empty=
echo "[${empty}]"
echo "[${missing-fallback}]"
echo "[${missing:-fallback}]"
echo "[${missing+alt}]"
set --
echo "[$@][$*][$#]"
echo done
