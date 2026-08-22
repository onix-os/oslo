#!/usr/bin/env sh
# Build oslo, for somebody who does not have oslo yet.
#
# `.make.lua` is the build — dependencies, staleness, parameters, the whole gate — but it is run by
# the `make` builtin, which is *inside the shell being built*. That is a bootstrap you cannot start
# from: a new checkout, a CI runner and a distribution packager all arrive with cargo and no oslo.
# This script is the one rung that gets you off the ground, and nothing more. Once `oslo` exists,
# `oslo make` is the build and this file has no second job.
#
#   scripts/build.sh                 static release, every feature
#   scripts/build.sh --minimal       static release, no optional features
#   scripts/build.sh --native        this machine's target, for a quick local binary
#
# POSIX `sh`, no bashisms: the machine that needs this is the one with the least on it.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

# The name and version come from the same place `.make.lua` reads them, so the two builds cannot
# disagree about what they are building.
meta=$(./scripts/project-meta.sh) || {
    echo "build: scripts/project-meta.sh failed; is this an oslo checkout?" >&2
    exit 1
}
name=$(echo "$meta" | sed -n 1p)
version=$(echo "$meta" | sed -n 2p)
[ -n "$name" ] || { echo "build: no project name in Cargo.toml" >&2; exit 1; }

# **A release is one file that runs anywhere.** oslo is meant to be somebody's login shell, and a
# login shell linked against a /nix/store glibc stops existing the day `nix-collect-garbage` runs —
# from inside the session it breaks, there is no recovering. So: musl, statically linked.
#
# Pointing this at `musl-gcc` silently produces a *dynamic* musl binary, which is why there is no
# linker override here. See `.github/workflows/release.yml`.
target=${TARGET:-x86_64-unknown-linux-musl}
features=--all-features
native=no

for arg in "$@"; do
    case $arg in
        --minimal) features= ;;
        --native)  native=yes ;;
        -h|--help)
            sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
            exit 0 ;;
        *)
            echo "build: $arg: unknown option; try --minimal, --native or --help" >&2
            exit 2 ;;
    esac
done

echo "$name $version"

if [ "$native" = yes ]; then
    # No `--target`, so cargo builds for the host and links against its libc. For an inner loop,
    # not for anything anybody else runs.
    # shellcheck disable=SC2086
    cargo build --release --bin "$name" $features
    bin="target/release/$name"
else
    command -v cargo >/dev/null 2>&1 || { echo "build: cargo is not installed" >&2; exit 1; }
    if ! rustup target list --installed 2>/dev/null | grep -qx "$target"; then
        # Only a warning: a nix shell or a distribution toolchain has the target without rustup
        # knowing anything about it, and refusing there would be wrong.
        echo "build: note: rustup does not list $target as installed; trying anyway" >&2
    fi
    # shellcheck disable=SC2086
    RUSTFLAGS="-C target-feature=+crt-static" \
        cargo build --release --target "$target" --bin "$name" $features
    bin="target/$target/release/$name"
fi

[ -f "$bin" ] || { echo "build: $bin was not produced" >&2; exit 1; }

# **"Static" is a claim about the ELF, so check the ELF.** `ldd` is not enough: it prints
# "statically linked" for a musl binary that still has an INTERP and will not start. Skipped for
# `--native`, which is dynamic on purpose.
if [ "$native" = no ] && command -v readelf >/dev/null 2>&1; then
    if readelf -l "$bin" | grep -q 'program interpreter'; then
        echo "build: $bin requests a dynamic loader; it is not static" >&2
        exit 1
    fi
    if readelf -d "$bin" 2>/dev/null | grep -q NEEDED; then
        echo "build: $bin has NEEDED entries; it is not static" >&2
        exit 1
    fi
    echo "static: no INTERP, no NEEDED"
fi

size=$(wc -c < "$bin")
# Bytes as well as megabytes: the README argues about kilobytes, and one release cannot be
# subtracted from the last if both are rounded to two decimal places.
awk -v b="$size" -v p="$bin" 'BEGIN { printf "%s  %.2f MB  (%d bytes)\n", p, b/1048576, b }'
echo
echo "Run it, or install it with:  $bin make install"
