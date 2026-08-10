#!/usr/bin/env bash
# Build the directory the demos run in.
#
# Fixed content, so a recording made today and one made after a refactor differ only by what the
# shell did. Everything lives under /tmp: a demo must never touch the repository it documents.
set -euo pipefail

WORK="${DEMO_WORK:-/tmp/oslo-demo-work}"
rm -rf "$WORK"
mkdir -p "$WORK"/{src,docs,build}

cat > "$WORK/README.md" <<'EOF'
# demo
A directory with enough in it to be worth looking at.
EOF

cat > "$WORK/Cargo.toml" <<'EOF'
[package]
name = "demo"
version = "0.1.0"
EOF

for n in main lib parser config; do
    printf 'fn %s() {\n    // …\n}\n' "$n" > "$WORK/src/$n.rs"
done
printf 'notes\n' > "$WORK/docs/notes.md"
printf 'log line\n' > "$WORK/build/output.log"
: > "$WORK/.hidden"

# A git worktree, so `cd root`, the prompt's branch segment and the tracking store all have
# something real to answer with.
if command -v git >/dev/null; then
    git -C "$WORK" init -q
    git -C "$WORK" add -A 2>/dev/null || true
    git -C "$WORK" -c user.email=demo@example.com -c user.name=demo \
        commit -qm "the demo tree" 2>/dev/null || true
fi

echo "$WORK"
