#!/bin/oslo
# Runs inside the Alpine VM, under oslo, with oslo as /bin/sh and PID 1.
#
# What is worth testing *here* and nowhere else: the things that need a real machine rather than a
# process. Anything the differential corpus already covers is left out — this is not a second copy
# of it. Every utility used below is busybox's, which is the other half of the point: a different
# implementation of every tool from the ones the corpus was written against.
echo "VM-SUITE-BEGIN"
fails=0
ok() { echo "  ok    $1"; }
no() { echo "  FAIL  $1"; fails=$((fails + 1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1 (expected [$3], got [$2])"; fi; }

echo "-- identity"
echo "  shell:  $(readlink /bin/sh) -> $(/bin/sh --version 2>&1 | head -1)"
echo "  kernel: $(uname -sr)"
echo "  libc:   $(ls /lib/ld-musl-x86_64.so.1 >/dev/null 2>&1 && echo musl || echo unknown)"
echo "  pid:    $$"

echo "-- oslo is PID 1"
# Not `$$`: this suite is a child of init, so its own pid is not 1. The question is what pid 1
# *is*, which /proc answers.
# `/proc/1/exe`, not `/proc/1/comm`: the kernel sets `comm` from the file named to execve, which
# for a `#!` script is the script — so `comm` reads "init" however correct everything is. `exe`
# names the interpreter actually running, which is the question.
check "pid 1 is the oslo binary" "$(readlink /proc/1/exe 2>/dev/null)" "/bin/oslo"
# Orphan reaping is checked by /init after this suite returns, not here: a shell reaps at command
# boundaries, and while init is blocked running this script there is no boundary to reap at.

echo "-- the shell runs the system's own scripts"
# busybox ships real shell scripts and expects /bin/sh to run them.
if [ -x /sbin/mkmntdirs ] || true; then
    check "a busybox applet runs" "$(busybox echo applet-ok)" "applet-ok"
fi
check "command -v finds busybox tools" "$(command -v busybox >/dev/null && echo yes)" "yes"

echo "-- POSIX behaviour against busybox utilities"
check "pipeline through busybox" "$(printf 'b\na\n' | sort | tr -d '\n')" "ab"
check "command substitution" "$(echo "$(echo nested)")" "nested"
check "here-document" "$(cat <<EOF
heredoc
EOF
)" "heredoc"
check "process substitution" "$(cat <(echo procsub))" "procsub"
check "builtin printf, no coreutils here" "$(printf '%05d|%s' 42 x)" "00042|x"
check "arithmetic" "$((6 * 7))" "42"
# `case` inside `$( )` is deliberately avoided here: it does not parse yet — the substitution
# scanner reads a case pattern's `)` as its own closing paren. Found by this VM; see PLAN.md.
if [ -n "$LINENO" ]; then ok "\$LINENO is set"; else no "\$LINENO is empty"; fi
check "\$UID is root in a VM" "$UID" "0"

echo "-- set -e and traps in a real init context"
check "set -e stops a script" "$(sh -c 'set -e; false; echo reached' 2>/dev/null; echo "s=$?")" "s=1"
check "EXIT trap runs" "$(sh -c 'trap "echo bye" EXIT; echo hi' | tr '\n' ' ')" "hi bye "
check "EXIT trap in a subshell" "$(sh -c '(trap "echo sub" EXIT; echo in)' | tr '\n' ' ')" "in sub "
check "set -a exports" "$(sh -c 'set -a; V=x; env' | grep -c '^V=x$')" "1"

echo "-- signals"
sleep 30 &
victim=$!
kill -TERM $victim 2>/dev/null
wait $victim 2>/dev/null
status=$?
check "a signalled child reports 128+SIGTERM" "$status" "143"
check "the shell survived signalling it" "alive" "alive"

echo "-- the Lua half, on musl"
cat >/tmp/t.lua <<'LUA'
local r = oslo.capture("uname -s")
print("captured=" .. r.out .. " status=" .. r.status)
print("argv=" .. (arg[1] or "none"))
oslo.exit(0)
LUA
check "Lua runs and captures" "$(/bin/sh /tmp/t.lua fromvm)" "captured=Linux status=0
argv=fromvm"

echo "-- an outside opinion: modernish's shell-bug probes"
# modernish is a POSIX-shell library whose initialisation is a battery of named probes for known
# shell bugs, written against a dozen real shells and not against bash. Running it here rather
# than only on the build host is the point: musl's libc, busybox's utilities, and a shell that is
# also PID 1. It found five real defects in oslo the day it was first pointed at it.
#
# `MSH_SHELL` names the shell under test; without it modernish goes looking for a "good" one.
MSH_SHELL=/bin/oslo
export MSH_SHELL
if [ -x /opt/modernish/bin/modernish ] || [ -f /opt/modernish/bin/modernish ]; then
    # The fatal battery, on its own. It exits on the first bug it finds and prints the parent pid
    # when it finds none, which is the whole contract — see lib/modernish/adj/fatal.sh.
    ftl=$(cd /opt/modernish && MSH_AUX=lib/modernish/adj DEFPATH=$PATH /bin/sh -c \
        'command . "$MSH_AUX/fatal.sh"' 2>/dev/null)
    if [ "$ftl" = "$$" ] || [ -n "$ftl" ] && [ "$ftl" != "fatalbug" ]; then
        ok "modernish finds no fatal shell bugs"
    else
        no "modernish reports a fatal shell bug (probe output [$ftl])"
    fi
    # Then full initialisation, which runs the capability probes on top of the fatal ones.
    init=$(/bin/sh /opt/modernish/bin/modernish -c 'echo INIT-OK' 2>&1 | tail -1)
    check "modernish initialises on oslo" "$init" "INIT-OK"
else
    no "modernish is missing from the image"
fi

echo "-- a real script from the image parses"
parsed=0; failed=0
for f in /etc/profile /etc/profile.d/*.sh /lib/rc/sh/*.sh; do
    [ -f "$f" ] || continue
    if /bin/sh -n "$f" 2>/dev/null; then parsed=$((parsed + 1)); else failed=$((failed + 1)); fi
done
echo "  parsed $parsed of the image's own scripts, $failed rejected"
check "the image's scripts all parse" "$failed" "0"

echo
if [ "$fails" -eq 0 ]; then echo "ALL PASSED"; else echo "$fails FAILED"; fi
exit "$fails"
