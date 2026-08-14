#!/usr/bin/env bash
# Build the directory the demos run in.
#
# Fixed content, so a recording made today and one made after a refactor differ only by what the
# shell did. Everything lives under /tmp: a demo must never touch the repository it documents.
set -euo pipefail

WORK="${DEMO_WORK:-/tmp/oslo-demo-work}"
rm -rf "$WORK"
mkdir -p "$WORK"/{src,docs,build}

# The two fixture machines, **beside `$WORK` rather than inside it**. `$WORK` is a git repository —
# the prompt and nav demos need one — and a machine's state directory under it puts the profile key
# inside a repository, which `oslo secret` correctly warns about on every call. The warning is right
# and the layout was wrong.
MACHINES="${WORK}-machines"
rm -rf "$MACHINES"

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

# A macro database of its own, under the work directory, so the manager demo shows a handful of
# invented macros rather than whatever the person recording happens to keep. Record that demo with
# `XDG_DATA_HOME="$WORK/data"` and the shell reads this store instead of the real one.
#
# Seeded through `oslo macros import`, which is the same door a person uses — so if the format
# changes and this stops working, that is worth knowing.
OSLO="${OSLO_BIN:-$PWD/target/x86_64-unknown-linux-musl/release/oslo}"
if [ -x "$OSLO" ]; then
    mkdir -p "$WORK/data"
    XDG_DATA_HOME="$WORK/data" "$OSLO" macros import >/dev/null <<'EOF'
alias gs #git
	git status --short --branch
alias gl #git
	git log --oneline --graph -20
abbrev gco #git
	git checkout
abbrev dc #docker
	docker compose
alias ports #net
	ss -tulpn
alias ips #net
	ip -c -brief addr
func mkcd #files
	mkdir -p "$1" && cd "$1"
script deploy #work
	#!/usr/bin/env python3
	import sys
	print("deploying", sys.argv[1:])
script backup #work
	#!/bin/sh
	rsync -a --delete "$1" /srv/backup/
var EDITOR #shell
	hx
var PAGER #shell
	bat
EOF
fi

# A secret, and a variable whose body goes and fetches it — the pair the `secrets` demo is about.
# Kept out of the import above because it is written through `oslo secret`, which is the door a
# person uses, and because a store seeded any other way would not have an identity beside it.
#
# `$WORK-key` rather than `$WORK/data`: the key does not live where the ciphertext lives, which is
# the whole arrangement — and outside the work tree, which is a git repository, or the shell would
# rightly say the key is one commit from being published.
if [ -x "$OSLO" ] && "$OSLO" secret --help >/dev/null 2>&1; then
    mkdir -p "$WORK/data" "$WORK-key"
    printf %s 'demo-not-a-real-github-token' |
        XDG_DATA_HOME="$WORK/data" XDG_STATE_HOME="$WORK-key" "$OSLO" secret set gh-token
    XDG_DATA_HOME="$WORK/data" XDG_STATE_HOME="$WORK-key" "$OSLO" macros add \
        --var 'GITHUB_TOKEN=$(oslo secret get gh-token)' --tag work >/dev/null
fi

# Two whole machines, for the `profile sync` act.
#
# **Both ends are fixtures.** The demo must never sync the recording machine's own history — it
# would merge invented commands into a real store — so `here` and `there` are two XDG homes under
# /tmp, each with a history of its own, and the shell in the recording steps into `here`.
if [ -x "$OSLO" ]; then
    for machine in here there; do
        mkdir -p "$MACHINES/$machine/state"
    done
    printf 'cargo build --release\ngit push origin main\nhx src/lib.rs\n' > "$MACHINES/here.txt"
    printf 'cargo build --release\nkubectl get pods -A\njournalctl -fu oslo\n' > "$MACHINES/there.txt"
    # **`OSLO_PROFILE=default` on every one of these.** The recorder runs under a profile of its own
    # so that nothing lands in a real history, and a fixture that inherited that name would seed one
    # profile and then be asked to sync a different one.
    for machine in here there; do
        OSLO_PROFILE=default \
            XDG_DATA_HOME="$MACHINES/$machine" XDG_STATE_HOME="$MACHINES/$machine/state" HOME="$MACHINES/$machine" \
            "$OSLO" history import "$MACHINES/$machine.txt" >/dev/null 2>&1
    done
    # The key that says the two are one profile, made on `here` and carried to `there` — the step a
    # person does once, done here so the recording can get to the part worth watching.
    OSLO_PROFILE=default \
        XDG_DATA_HOME="$MACHINES/here" XDG_STATE_HOME="$MACHINES/here/state" HOME="$MACHINES/here" \
        "$OSLO" profile key init default >/dev/null 2>&1
    OSLO_PROFILE=default \
        XDG_DATA_HOME="$MACHINES/here" XDG_STATE_HOME="$MACHINES/here/state" HOME="$MACHINES/here" \
        "$OSLO" profile export default 2>/dev/null |
        OSLO_PROFILE=default \
            XDG_DATA_HOME="$MACHINES/there" XDG_STATE_HOME="$MACHINES/there/state" HOME="$MACHINES/there" \
            "$OSLO" profile import default >/dev/null 2>&1

    # A macro and a secret on each, so the sync demo has all three parts to carry rather than one.
    # Seeded through `macros import` because a function and a script would otherwise want an editor.
    printf 'alias gs\n\tgit status --short\nscript deploy\n\t#!/bin/sh\n\techo shipping\n' \
        > "$MACHINES/here-macros.txt"
    printf 'alias kp\n\tkubectl get pods -A\n' > "$MACHINES/there-macros.txt"
    for machine in here there; do
        OSLO_PROFILE=default \
            XDG_DATA_HOME="$MACHINES/$machine" XDG_STATE_HOME="$MACHINES/$machine/state" HOME="$MACHINES/$machine" \
            "$OSLO" macros import "$MACHINES/$machine-macros.txt" >/dev/null 2>&1
    done
    printf 'sk-not-a-real-one' |
        OSLO_PROFILE=default \
            XDG_DATA_HOME="$MACHINES/here" XDG_STATE_HOME="$MACHINES/here/state" HOME="$MACHINES/here" \
            "$OSLO" secret set deploy-token >/dev/null 2>&1
    printf 'also-invented' |
        OSLO_PROFILE=default \
            XDG_DATA_HOME="$MACHINES/there" XDG_STATE_HOME="$MACHINES/there/state" HOME="$MACHINES/there" \
            "$OSLO" secret set registry >/dev/null 2>&1
fi

# A stand-in for `ssh`, so the sync act has a far end without a second computer.
#
# **Named so nobody mistakes it for ssh.** It ignores the destination and runs the command against
# the `there` machine — which is exactly what ssh would do, minus the network. The demo shows the
# script on screen, because a recording that implied a real remote host would be lying about the
# one thing being demonstrated.
mkdir -p "$WORK/bin"

# The near end, for the same reason.
#
# **A prefix rather than a nested shell.** `oslo` typed at an oslo prompt asks whether you meant to
# nest — see `startup/nested.rs` — and a recording cannot answer a question that only appears at
# some depths. This runs one command as if on the machine called `here`, and needs no shell.
cat > "$WORK/bin/here" <<EOF
#!/bin/sh
# stands in for: being logged in on the machine called \`here\`
exec env OSLO_PROFILE=default \\
    XDG_DATA_HOME=$MACHINES/here XDG_STATE_HOME=$MACHINES/here/state HOME=$MACHINES/here "\$@"
EOF
chmod +x "$WORK/bin/here"

cat > "$WORK/bin/pretend-ssh" <<EOF
#!/bin/sh
# stands in for: ssh USER@HOST oslo …
shift
exec env OSLO_PROFILE=default \\
    XDG_DATA_HOME=$MACHINES/there XDG_STATE_HOME=$MACHINES/there/state HOME=$MACHINES/there "\$@"
EOF
chmod +x "$WORK/bin/pretend-ssh"

# A second machine's published half, for the `recipient add` beat.
#
# Made with the binary under test in a home of its own, so it is a *real* recipient and the demo
# exercises the same validation a person's would — an invented string would be refused on camera.
if [ -x "$OSLO" ] && "$OSLO" secret --help >/dev/null 2>&1; then
    XDG_DATA_HOME="$WORK/elsewhere" XDG_STATE_HOME="$WORK/elsewhere-key" \
        "$OSLO" secret key init 2>/dev/null | tail -1 > "$WORK/build-server.pub"
    printf '# the build server\n' | cat - "$WORK/build-server.pub" > "$WORK/tmp.pub" \
        && mv "$WORK/tmp.pub" "$WORK/build-server.pub"
fi

# A stand-in for a program that does a store's crypto, for the `secrets` demo.
#
# **Not `age`, and named so nobody mistakes it for it.** The machine these are recorded on has
# neither `age` nor a YubiKey, and a recording that pretended otherwise would be the one kind of
# dishonesty a demo cannot recover from. What it does show is the contract, which is the whole of
# what oslo relies on: bytes in on standard input, bytes out on standard output, and a prompt on
# standard error the way a hardware key asks for a touch.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/pretend-age" <<'EOF'
#!/bin/sh
# stands in for: age -R recipients.txt   /   age --decrypt --identity yubikey.txt
echo "touch your key" >&2
case "$1" in
  -e) printf 'SEALED\n'; base64 ;;
  -d) tail -n +2 | base64 -d ;;
esac
EOF
chmod +x "$WORK/bin/pretend-age"


# Two scripts that declare their arguments in comments, for the `argc` demos: one oslo, one bash.
# They live in `$WORK/bin`, which those demos put on `$PATH`, so the shell finds them by name and
# completes them the way it would any other command.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/deploy" <<'EOF'
#!/usr/bin/env oslo
# @describe        Send a build somewhere
# @flag   -n --dry-run          say what would happen, do nothing
# @option -t --tries <N>        how many times to try
# @option -e --env[dev|staging|prod]   which environment
# @arg    build!                the build to send
argc "$@"

echo "sending $argc_build to ${argc_env:-dev} ($argc_tries tries, dry=${argc_dry_run:-0})"
EOF

cat > "$WORK/bin/release" <<'EOF'
#!/usr/bin/env bash
# @describe        Cut a release
# @flag   -f --force            even with a dirty tree
# @option -m --message <TEXT>   the tag message
# @arg    version!              the version to cut
eval "$(oslo --argc-eval "$0" "$@")"

echo "cutting $argc_version (force=${argc_force:-0}) — $argc_message"
EOF
chmod +x "$WORK/bin/deploy" "$WORK/bin/release"

echo "$WORK"
