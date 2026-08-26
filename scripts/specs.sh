#!/usr/bin/env sh
# Regenerate `config/specs` from the two upstream completion corpora.
#
#   scripts/specs.sh                 refresh config/specs from both corpora
#   scripts/specs.sh --with-giants   include aws and gcloud as well
#   scripts/specs.sh --out DIR       write somewhere else
#
# The result is committed, so this is run when upstream moves rather than on every build. Nothing
# in oslo's build depends on it, and a checkout that never runs it still has the specs.
#
# # Why two corpora and which one wins
#
# They overlap on 367 commands and disagree about almost nothing except emphasis. Fig's are
# hand-written and carry value lists — `git checkout <branch>` suggests real branch names, `docker
# run --network` names the real networks — where argc's are generated from each tool's own `--help`
# and are broader but flatter: every dynamic choice in them is a bash function, which a spec file
# cannot hold. So **Fig wins where both exist** and argc fills in the 474 commands Fig has never
# heard of.
#
# # Why aws and gcloud are left out by default
#
# They are 45MB of the 57MB Fig converts to, and 3.6MB of packed repository against 2.2MB for the
# other 1,192 commands put together. Two commands should not cost more than everything else does.
# `--with-giants` puts them back for somebody who wants them locally.

set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

cache="${OSLO_SPEC_CACHE:-${TMPDIR:-/tmp}/oslo-spec-corpora}"
out="$root/config/specs"
giants=no

while [ $# -gt 0 ]; do
    case $1 in
        --with-giants) giants=yes ;;
        --out) shift; out=$1 ;;
        -h|--help) sed -n '2,8p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "specs: $1: unknown option; try --with-giants, --out or --help" >&2; exit 2 ;;
    esac
    shift
done

command -v git >/dev/null 2>&1 || { echo "specs: git is not installed" >&2; exit 1; }
command -v bun >/dev/null 2>&1 || {
    echo "specs: bun is not installed — it is what reads Fig's TypeScript specs." >&2
    echo "       See https://bun.sh, or run with only the argc half by removing fig below." >&2
    exit 1
}

# Shallow, and refreshed in place: these are large repositories and only their current state
# matters. A conversion is reproducible from a commit, not from a history.
fetch() {
    dir="$cache/$1"
    if [ -d "$dir/.git" ]; then
        echo "specs: refreshing $1"
        git -C "$dir" fetch --depth 1 origin HEAD >/dev/null 2>&1 || true
        git -C "$dir" reset --hard FETCH_HEAD >/dev/null 2>&1 || true
    else
        echo "specs: cloning $1"
        mkdir -p "$cache"
        git clone --depth 1 -q "$2" "$dir"
    fi
}

mkdir -p "$cache"
fetch fig https://github.com/withfig/autocomplete.git
fetch argc https://github.com/sigoden/argc-completions.git

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "specs: converting Fig's TypeScript"
bun "$root/scripts/fig-to-spec.ts" "$cache/fig/src" "$work/fig"

echo "specs: converting argc's shell scripts"
cargo run -q --release -p argc-to-spec -- "$cache/argc/completions" "$work/argc"

# argc first, then Fig over the top: the last writer wins, and Fig is the one that should.
echo "specs: merging into $out"
rm -rf "$out"
mkdir -p "$out"
cp "$work/argc/"*.yaml "$out/" 2>/dev/null || true
cp "$work/fig/"*.yaml "$out/" 2>/dev/null || true

if [ "$giants" = no ]; then
    rm -f "$out/aws.yaml" "$out/gcloud.yaml"
fi

count=$(find "$out" -name '*.yaml' -type f | wc -l | tr -d ' ')
size=$(du -sh "$out" | cut -f1)
echo "specs: $count commands, $size"
