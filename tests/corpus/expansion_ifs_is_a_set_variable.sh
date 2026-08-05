# mode: posix
# POSIX: the shell sets IFS at startup to <space><tab><newline>. It is a real variable, not just a
# default the splitter falls back to — `${#IFS}` is 3, `${IFS+SET}` is non-empty, and `$IFS` under
# `set -u` is not an error.
#
# Found by running every `#!/bin/sh` script on a Debian system: `/usr/bin/xdg-terminal-exec` opens
# with `XTE__OIFS=$IFS` under `set -u` and died on "IFS: unbound variable", because oslo defaulted
# the value where it read it and never set the variable at all.
echo "len=${#IFS}"
echo "set=${IFS+SET}"
echo "unset-branch=${IFS-NOT-SET}"

# The save-and-restore idiom, which is the reason this has to be a variable.
saved=$IFS
IFS=:
set -- one:two:three
echo "split=$#"
IFS=$saved
set -- a b c
echo "restored=$#"

# Not exported: a child has no use for its parent's field separators, and neither bash nor dash
# lists it in `export -p`.
export -p | grep -c '^export IFS' || true

# And it is still an ordinary variable that a script may set.
IFS=,
echo "assigned=[$IFS]"
