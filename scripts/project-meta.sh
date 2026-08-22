#!/usr/bin/env sh
# Prints the project name and version, one per line, read from Cargo.toml.
#
# Cargo.toml is the only place either of them can be wrong in a way that matters: it is what the
# binary is built from. A second file repeating them is a second thing to forget, and it stayed
# right only by luck — so it is gone, and this reads the manifest instead.
#
# A script rather than inline in either build, because both read it: `.make.lua` and the bootstrap
# skip comment lines), and GNU Make 3.81 treats a `#` inside `$(shell ...)` as the start of a Make
# comment. It truncates the call and reports `unterminated call to function 'shell': missing ')'`,
# which fires for *every* target and looks like the whole gate failing for no stated reason.
# Make 4.x parses the same line correctly, so the bug only shows on an old make.
set -eu

cd "$(dirname "$0")/.."

[ -f Cargo.toml ] || exit 0

# Only the `[package]` and `[workspace.package]` sections: a dependency called `name` further down
# the file would otherwise be picked up and quietly rename the project. Trailing \r is stripped so
# a checkout with CRLF endings does not produce a name that looks right and compares unequal
# everywhere it is used.
#
# **The version may be inherited rather than stated.** Every crate in this tree takes its version
# from `[workspace.package]`, so `[package]` says `version.workspace = true` and the number itself
# is one line further down. Read from `[package]` first — a crate that states its own still wins —
# and fall back to the workspace's. Without the fallback this printed nothing, and the name and
# version are what every `make` target puts in its banner.
sed -e 's/\r$//' Cargo.toml |
    awk '
        /^\[/            { in_package = ($0 == "[package]");
                           in_workspace = ($0 == "[workspace.package]"); next }
        /^[[:space:]]*#/ { next }
        in_package       { if ($1 == "name")    name = $3
                           if ($1 == "version") version = $3
                           next }
        in_workspace     { if ($1 == "version") shared = $3
                           next }
        END              { if (version == "") version = shared
                           gsub(/"/, "", name); gsub(/"/, "", version);
                           if (name != "") print name; if (version != "") print version }
    '
