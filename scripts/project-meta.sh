#!/usr/bin/env sh
# Prints the project name and version, one per line, read from Cargo.toml.
#
# Cargo.toml is the only place either of them can be wrong in a way that matters: it is what the
# binary is built from. A second file repeating them is a second thing to forget, and it stayed
# right only by luck — so it is gone, and this reads the manifest instead.
#
# This lives in a script rather than inline in the Makefile because the parsing needs a `#` (to
# skip comment lines), and GNU Make 3.81 treats a `#` inside `$(shell ...)` as the start of a Make
# comment. It truncates the call and reports `unterminated call to function 'shell': missing ')'`,
# which fires for *every* target and looks like the whole gate failing for no stated reason.
# Make 4.x parses the same line correctly, so the bug only shows on an old make.
set -eu

cd "$(dirname "$0")/.."

[ -f Cargo.toml ] || exit 0

# Only the `[package]` section: a dependency called `name` further down the file would otherwise
# be picked up and quietly rename the project. Trailing \r is stripped so a checkout with CRLF
# endings does not produce a name that looks right and compares unequal everywhere it is used.
sed -e 's/\r$//' Cargo.toml |
    awk '
        /^\[/            { in_package = ($0 == "[package]"); next }
        !in_package      { next }
        /^[[:space:]]*#/ { next }
        $1 == "name"     { name = $3 }
        $1 == "version"  { version = $3 }
        END              { gsub(/"/, "", name); gsub(/"/, "", version);
                           if (name != "") print name; if (version != "") print version }
    '
