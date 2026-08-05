#!/usr/bin/env bash
# What oslo does when it is root, and when it is reached through sudo.
#
# **Read-only.** Nothing here edits /etc/passwd, installs anything, or runs chsh. Every store it
# writes goes to a temporary directory that is deleted at the end, so the one real risk — root
# writing into your own history — is *detected* rather than demonstrated.
#
# Run it:  bash test_root_and_sudo.sh
# It asks for your sudo password once, at the start.
#
# Delete this file when you are done with it; it is a diagnostic, not part of the project.

set -uo pipefail

O=${OSLO:-$(cd "$(dirname "$0")" && pwd)/target/x86_64-unknown-linux-musl/release/oslo}
[ -x "$O" ] || O=$(cd "$(dirname "$0")" && pwd)/target/debug/oslo
scratch=$(mktemp -d)
# **Cleaned up as root when it has to be.** Root writes its store here with mode 700 root:root, so
# the ordinary `rm` cannot remove it and the directory is left behind in /tmp for ever. The `sudo`
# fallback runs only if the plain one fails, and only on a path this script made.
cleanup() {
    rm -rf "$scratch" 2>/dev/null || sudo rm -rf "$scratch" 2>/dev/null
}
trap cleanup EXIT

pass=0
warn=0
note() { printf '  \033[38;5;8m%s\033[0m\n' "$*"; }
ok()   { printf '  \033[38;5;2mok\033[0m    %s\n' "$*"; pass=$((pass+1)); }
bad()  { printf '  \033[38;5;1mWARN\033[0m  %s\n' "$*"; warn=$((warn+1)); }
head() { printf '\n\033[1;38;5;3m%s\033[0m\n' "$*"; }

head "BINARY"
if [ ! -x "$O" ]; then
    echo "  no oslo binary at $O — run 'make build' first" >&2
    exit 1
fi
note "$O"
note "$(file -b "$O" | cut -c1-80)"
"$O" --version | sed 's/^/  /'

head "SUDO"
if ! sudo -v 2>/dev/null; then
    echo "  cannot get sudo; the rest needs it" >&2
    exit 1
fi
sudo_home=$(sudo printenv HOME 2>/dev/null)
note "sudo hands over HOME=$sudo_home"
if [ "$sudo_home" = "/root" ]; then
    ok "root gets its own history; yours is untouched"
else
    bad "root would write into YOUR store ($sudo_home/.local/share/oslo)"
    note "fix: echo 'Defaults always_set_home' | sudo tee /etc/sudoers.d/always_set_home"
    note "or:  put OSLO_PROFILE=root in root's environment"
fi
note "sudo -i hands over HOME=$(sudo -i printenv HOME 2>/dev/null)"

head "RUNNING AS ROOT"
printf '  id:      '; sudo "$O" -c 'echo "uid=$(id -u) euid=$(id -u) home=$HOME"'
printf '  arith:   '; sudo "$O" -c 'echo $((6*7))'
printf '  IFS set: '; sudo "$O" -c 'echo ${#IFS}'
printf '  pipes:   '; sudo "$O" -c 'printf "b\na\n" | sort | tr "\n" " "'; echo
printf '  trap:    '; sudo "$O" -c 'trap "echo trapped" EXIT; true'
printf '  status:  '; sudo "$O" -c 'exit 42'; echo "$?"

head "ROOT LOGIN SHELL"
note "reads /etc/profile then /root/.profile"
sudo test -f /root/.profile && note "/root/.profile exists" || note "/root/.profile does not exist (fine)"
before=$(sudo "$O" -c 'echo "$PATH"')
after=$(sudo "$O" -l -c 'echo "$PATH"')
if [ "$before" != "$after" ]; then
    ok "-l changed PATH, so /etc/profile ran"
    note "without -l: $(echo "$before" | cut -c1-60)..."
    note "with    -l: $(echo "$after"  | cut -c1-60)..."
else
    note "PATH unchanged by -l (this system's /etc/profile may not set it for root)"
fi
printf '  /etc/profile.d ran: '
sudo "$O" -l -c 'case "$PATH" in *snap*|*games*) echo yes;; *) echo "cannot tell from PATH";; esac'

head "WHERE ROOT WRITES ITS HISTORY"
# Into the scratch directory, never the real one.
sudo env XDG_DATA_HOME="$scratch/rootdata" HISTFILE="$scratch/roothist" OSLO_ALLHIST=1 \
    "$O" -c 'echo a-root-command' >/dev/null 2>&1
if sudo test -f "$scratch/roothist"; then
    ok "root's \$HISTFILE was written"
    note "owner: $(sudo stat -c '%U:%G %a' "$scratch/roothist")"
else
    bad "root could not write a history file"
fi
if sudo test -f "$scratch/rootdata/oslo/default.kv"; then
    ok "root's tracking store was written"
    note "owner: $(sudo stat -c '%U:%G %a' "$scratch/rootdata/oslo/default.kv")"
    note "dir:   $(sudo stat -c '%U:%G %a' "$scratch/rootdata/oslo")"
else
    bad "root could not write a tracking store"
fi

head "YOUR OWN FILES — DID ANYTHING ROOT-OWNED APPEAR?"
found=$(find "$HOME/.local/share/oslo" "$HOME/.oslo_history" -user root 2>/dev/null)
if [ -z "$found" ]; then
    ok "nothing in your history is owned by root"
else
    bad "root-owned files in your history:"
    echo "$found" | sed 's/^/        /'
    note "fix: sudo chown -R $USER $found"
fi

head "SCRIPTS RUN AS ROOT (the apt/dpkg case)"
cat > "$scratch/maint.sh" <<'EOF'
#!/bin/sh
set -e
[ "$(id -u)" = "0" ] || { echo "not root"; exit 1; }
tmp=$(mktemp); echo data > "$tmp"; grep -q data "$tmp" && rm -f "$tmp"
echo "maintainer-script-ok"
EOF
chmod +x "$scratch/maint.sh"
printf '  as a script:  '; sudo "$O" "$scratch/maint.sh"
printf '  via shebang:  '; sudo sh -c "sed -i '1s|.*|#!$O|' '$scratch/maint.sh'; '$scratch/maint.sh'"

head "SUMMARY"
printf '  %d ok, %d warnings\n' "$pass" "$warn"
[ "$warn" -eq 0 ] && echo "  nothing to act on." || echo "  see the WARN lines above."
