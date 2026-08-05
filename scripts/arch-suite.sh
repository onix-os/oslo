#!/bin/sh
# What to ask of oslo once it *is* `/bin/sh` on a real Arch userland.
#
# Arch is the harder of the two distro tests, and deliberately so. Alpine's `/bin/sh` is busybox
# ash and Debian's is dash, both small POSIX shells; **Arch's is bash**, so every `#!/bin/sh`
# script the distro ships was written against bash and is allowed to use bashisms. Replacing it
# means being bash-compatible, not merely POSIX-correct.
#
# Run by `arch-vm.sh` inside the VM. Prints one line per check and a final count; the harness
# outside reads `ARCH-SUITE-EXIT:<n>`.

fail=0
ok() { printf '  ok    %s\n' "$1"; }
no() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1 (want [$3], got [$2])"; fi; }

echo "ARCH-SUITE-BEGIN"

# ---------------------------------------------------------------- we really are the system shell
check "/bin/sh is oslo" "$(readlink -f /bin/sh | sed 's|.*/||')" "oslo"
check "sh -c works" "$(sh -c 'echo alive')" "alive"
check "\$0 through sh -c" "$(sh -c 'echo $0' myname)" "myname"

# A shebang script is the case that matters: the kernel runs `/bin/sh`, not a shell we chose.
cat > /tmp/shebang.sh <<'EOF'
#!/bin/sh
echo "shebang:$1"
EOF
chmod +x /tmp/shebang.sh
check "shebang scripts run" "$(/tmp/shebang.sh arg)" "shebang:arg"

# ------------------------------------------------------------------------------- POSIX behaviour
check "IFS is set" "${#IFS}" "3"
check "arithmetic" "$((6 * 7))" "42"
check "parameter expansion" "${undefined:-fallback}" "fallback"
check "command substitution" "$(echo nested $(echo deep))" "nested deep"
check "pipelines" "$(printf 'b\na\n' | sort | tr '\n' ' ')" "a b "
check "here-documents" "$(cat <<EOF
heredoc
EOF
)" "heredoc"
check "functions and locals" "$(f() { x=inner; echo "$x"; }; f)" "inner"
check "traps" "$(sh -c 'trap "echo trapped" EXIT; true')" "trapped"
check "exit status" "$(sh -c 'exit 42'; echo $?)" "42"

# ------------------------------------------------------------ the distro's own scripts, in bulk
#
# Syntax-checking every `#!/bin/sh` script on the image is the closest thing to a system-wide test
# there is, and it is what found three real bugs on Debian. Compared against bash, which is what
# this system's `/bin/sh` was before oslo replaced it.
echo
echo "  scanning the image's own #!/bin/sh scripts..."
total=0
agree=0
differ=""
for d in /usr/bin /usr/sbin /etc /usr/lib/systemd; do
    [ -d "$d" ] || continue
    for f in $(find "$d" -maxdepth 3 -type f 2>/dev/null); do
        head -c 30 "$f" 2>/dev/null | head -1 | grep -qE '^#! ?/(usr/)?bin/sh' || continue
        total=$((total + 1))
        sh -n "$f" 2>/dev/null
        mine=$?
        bash -n "$f" 2>/dev/null
        theirs=$?
        if [ "$mine" = "$theirs" ]; then
            agree=$((agree + 1))
        else
            differ="$differ $f"
        fi
    done
done
echo "  parsed $agree of $total the same way bash does"
if [ "$agree" != "$total" ]; then
    no "these parse differently from bash:$differ"
else
    ok "every #!/bin/sh script on the image agrees with bash"
fi

# **Running them is the stronger test**, and it is the one that found three real bugs on Debian:
# `$IFS` being unset, a bare `exit` in an EXIT trap, and a comment inside a heredoc substitution.
# Parsing proves the grammar; only running proves the semantics.
#
# `--help` and `--version` are the two arguments almost every script answers and almost none acts
# on, which is what makes them safe to run as root on a live system.
echo
echo "  running them, comparing stdout and exit status against bash..."
runs=0
same=0
rundiff=""
for d in /usr/bin /usr/sbin /etc /usr/lib/systemd; do
    [ -d "$d" ] || continue
    for f in $(find "$d" -maxdepth 3 -type f 2>/dev/null); do
        head -c 30 "$f" 2>/dev/null | head -1 | grep -qE '^#! ?/(usr/)?bin/sh' || continue
        for arg in --help --version; do
            runs=$((runs + 1))
            mine=$(timeout 5 sh "$f" $arg </dev/null 2>/dev/null); mc=$?
            theirs=$(timeout 5 bash "$f" $arg </dev/null 2>/dev/null); tc=$?
            if [ "$mine" = "$theirs" ] && [ "$mc" = "$tc" ]; then
                same=$((same + 1))
            else
                rundiff="$rundiff $f($arg)"
            fi
        done
    done
done
echo "  $same of $runs runs matched bash exactly"
if [ "$same" != "$runs" ]; then
    no "these behaved differently from bash:$rundiff"
else
    ok "every run matched bash"
fi

# ------------------------------------------------------------------ pacman, the thing that hurts
#
# Package management runs `#!/bin/sh` as root, and it is what you would need in order to *undo*
# putting oslo here. `--version` and a database query exercise its own shelling out.
if command -v pacman >/dev/null 2>&1; then
    pacman -V >/dev/null 2>&1 && ok "pacman runs" || no "pacman runs"
    pacman -Qq >/dev/null 2>&1 && ok "pacman can read its database" || no "pacman can read its database"
fi

echo
echo "  $fail failed"
echo "ARCH-SUITE-EXIT:$fail"
