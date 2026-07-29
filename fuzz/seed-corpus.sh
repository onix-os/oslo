#!/usr/bin/env bash
# Build fuzz/corpus/<target>/ from material already in the repository.
#
# Random bytes take a long time to discover that a shell has a `case` statement. The 375 scripts
# under tests/corpus are real programs written to exercise real constructs, so they are a far
# better starting point than anything a mutator would find on its own — and they cost nothing,
# because they are already committed and already maintained by the differential suite.
#
# The generated corpus is not committed (fuzz/.gitignore). What is committed is fuzz/seeds/:
# hand-written inputs for the shapes no corpus script contains, plus any crash reproducer a run
# turns up. Regenerating is idempotent; run it again after adding a seed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/.." && pwd)"
scripts="$repo/tests/corpus"
out="$here/corpus"

if [ ! -d "$scripts" ]; then
    echo "no shell corpus at $scripts" >&2
    exit 1
fi

mkdir -p "$out/fuzz_parse" "$out/fuzz_lexer" "$out/fuzz_arith"

# Name every generated file after a hash of its contents. Two inputs that are the same input get
# one file, and re-running the script never produces a second copy under a different name.
emit() {
    local dir="$1" body="$2" name
    [ -n "${body//[[:space:]]/}" ] || return 0
    name="$(printf '%s' "$body" | cksum | tr -d ' \n')"
    printf '%s' "$body" > "$dir/$name"
}

# The parser and the word lexer both take whole-file input: a script is a script, and the word
# lexer's job on a script is to find the first word in it, which is exactly the entry rush uses.
for script in "$scripts"/*.sh; do
    cp -f "$script" "$out/fuzz_parse/$(basename "$script")"
    cp -f "$script" "$out/fuzz_lexer/$(basename "$script")"
done

# Arithmetic is different: a whole script is not an expression, and feeding one in only ever
# exercises the tokeniser's first rejection. Pull the expression bodies out instead — `$(( … ))`
# expansions, `(( … ))` commands and `let` arguments — so the target starts from expressions the
# shell is actually asked to evaluate.
while IFS= read -r expr; do
    emit "$out/fuzz_arith" "${expr#\$((}"
done < <(grep -hoE '\$\(\([^()]*\)\)' "$scripts"/*.sh | sed 's/))$//' || true)

while IFS= read -r expr; do
    emit "$out/fuzz_arith" "$expr"
done < <(grep -hoE '^[[:space:]]*\(\([^()]*\)\)' "$scripts"/*.sh | sed -E 's/^[[:space:]]*\(\(//; s/\)\)$//' || true)

# Committed seeds go in last so a hand-written input always wins the filename.
for target in fuzz_parse fuzz_lexer fuzz_arith; do
    [ -d "$here/seeds/$target" ] || continue
    cp -f "$here/seeds/$target"/* "$out/$target/" 2>/dev/null || true
done

for target in fuzz_parse fuzz_lexer fuzz_arith; do
    printf '%s: %d inputs\n' "$target" "$(find "$out/$target" -type f | wc -l)"
done
