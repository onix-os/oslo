#!/usr/bin/env sh
# Prints the project name and version from the PROJECT file, one per line.
#
# This lives in a script rather than inline in the Makefile because the parsing needs a `#` (to
# skip comment lines), and GNU Make 3.81 treats a `#` inside `$(shell ...)` as the start of a Make
# comment. It truncates the call and reports `unterminated call to function 'shell': missing ')'`,
# which fires for *every* target and looks like the whole gate failing for no stated reason.
# Make 4.x parses the same line correctly, so the bug only shows on an old make.
#
# Format of PROJECT: name on the first content line, version on the second. Blank lines, `#`
# comments, and `[section]` headers are ignored.
set -eu

cd "$(dirname "$0")/.."

[ -f PROJECT ] || exit 0

# Trailing \r is stripped so a file checked out with CRLF endings does not produce a name that
# looks right and compares unequal everywhere it is used.
sed -e 's/\r$//' PROJECT |
    grep -v '^[[:space:]]*#' |
    grep -v '^[[:space:]]*\[' |
    grep -v '^[[:space:]]*$' |
    sed -e 's/[[:blank:]]//g' |
    head -2
