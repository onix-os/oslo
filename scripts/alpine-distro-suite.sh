#!/bin/oslo
# Runs inside the Alpine+OpenRC VM, after the init system has brought the system up.
#
# The question this answers is narrower than "does oslo work" and much more useful: *did other
# people's shell run on it*. Every assertion below is about code nobody on this project wrote —
# OpenRC's runtime, Alpine's service scripts, `alpine-conf`'s setup tools.
echo "DISTRO-SUITE-BEGIN"
fails=0
ok() { echo "  ok    $1"; }
no() { echo "  FAIL  $1"; fails=$((fails + 1)); }
check() { if [ "$2" = "$3" ]; then ok "$1"; else no "$1 (expected [$3], got [$2])"; fi; }

echo "-- identity"
echo "  shell:  $(readlink /bin/sh) -> $(/bin/sh --version 2>&1 | head -1)"
echo "  kernel: $(uname -sr)"
echo "  openrc: $(openrc --version 2>&1 | head -1)"

echo "-- OpenRC ran on oslo"
# `rc-status` is itself a shell script, and it reads the state OpenRC just wrote.
rc-status --servicelist >/tmp/services 2>/dev/null
check "rc-status runs" "$?" "0"
if [ -s /tmp/services ]; then
    ok "rc-status names services ($(wc -l </tmp/services) of them)"
else
    no "rc-status produced nothing"
fi
check "OpenRC recorded a runlevel" "$(cat /run/openrc/softlevel 2>/dev/null)" "default"

echo "-- a service script, run by hand under oslo"
# Starting a service exercises the whole of `/lib/rc/sh`: functions.sh, rc-functions.sh, the
# option parser and the service's own body. `hostname` is chosen because it is pure shell and its
# effect is observable without a daemon.
/etc/init.d/hostname start >/tmp/hostname.out 2>&1
hostname_status=$?
check "the hostname service starts" "$hostname_status" "0"
[ -s /tmp/hostname.out ] && echo "  (it said: $(head -2 /tmp/hostname.out | tr '\n' ' '))"

if [ -f /etc/init.d/bootmisc ]; then
    /etc/init.d/bootmisc describe >/dev/null 2>&1
    check "a service answers 'describe'" "$?" "0"
fi

echo "-- OpenRC's own shell runtime"
# These are the files every service sources. Running them is a much stronger test than parsing:
# they define functions, set variables from `/etc/rc.conf`, and branch on shell features.
# Alpine moved these from /lib/rc/sh to /usr/libexec/rc/sh; look in both, and complain if
# neither exists rather than silently checking nothing — the first version of this section found
# no files at all and reported success.
rcsh=
for dir in /usr/libexec/rc/sh /lib/rc/sh; do
    [ -d "$dir" ] && rcsh=$dir && break
done
if [ -z "$rcsh" ]; then
    no "OpenRC's shell runtime is not where this test looks"
else
    echo "  (runtime lives in $rcsh)"
    runtime_failed=0
    for f in "$rcsh"/*.sh; do
        [ -f "$f" ] || continue
        /bin/sh -n "$f" 2>/dev/null || {
            runtime_failed=$((runtime_failed + 1))
            echo "    rejected: $f"
        }
    done
    check "every file of OpenRC's shell runtime parses" "$runtime_failed" "0"
    # Sourcing is a much stronger test than parsing: functions.sh defines the helpers every
    # service calls, reads /etc/rc.conf, and branches on shell features while doing it.
    (. "$rcsh/functions.sh" && command -v einfo >/dev/null) 2>/dev/null
    check "sourcing functions.sh defines einfo" "$?" "0"
fi

echo "-- the whole image's shell, parsed"
# The reason this VM exists. The minirootfs had two shell scripts; a real Alpine has scores.
#
# Found by walking the filesystem rather than by a list of globs. A hand-written list only ever
# covers what someone thought of, and reports a confident number either way — the first version of
# this sweep missed OpenRC's entire runtime because Alpine had moved it, and said "all parse".
: >/tmp/rejected
scripts=$(find / -xdev -type f \
    ! -path '/proc/*' ! -path '/sys/*' ! -path '/dev/*' ! -path '/tmp/*' 2>/dev/null)

parsed=0
failed=0
for f in $scripts; do
    # Only files a shell is meant to read. `openrc-run` is OpenRC's own interpreter line and the
    # scripts carrying it are POSIX shell with extra functions defined around them.
    head -1 "$f" 2>/dev/null | grep -qE '^#!.*(/sh|/bash|openrc-run)' || continue
    if /bin/sh -n "$f" 2>>/tmp/rejected; then
        parsed=$((parsed + 1))
    else
        failed=$((failed + 1))
        echo "    rejected: $f" >>/tmp/rejected
    fi
done
echo "  parsed $parsed of the image's own shell scripts, $failed rejected"

# A sweep that found nothing, or that would accept anything, is worse than no sweep: it reports
# success either way. Both halves are asserted before the result is believed.
if [ "$parsed" -lt 50 ]; then
    no "the sweep found only $parsed scripts; it is not looking where the distro keeps them"
fi
printf 'if then fi\n' >/tmp/notshell.sh
if /bin/sh -n /tmp/notshell.sh 2>/dev/null; then
    no "'sh -n' accepts a syntax error, so this whole sweep proves nothing"
else
    ok "'sh -n' rejects a known-bad script, so the sweep means something"
fi

if [ "$failed" -ne 0 ]; then
    no "$failed of the distro's scripts do not parse"
    head -20 /tmp/rejected
else
    ok "every shell script in the image parses"
fi

echo "-- busybox utilities under oslo, as a distro uses them"
check "pipelines" "$(printf 'b\na\n' | sort | head -1)" "a"
check "command substitution in a service idiom" "$(basename "$(readlink -f /bin/sh)")" "oslo"
check "test builtin agrees with the system" "$([ -d /etc/init.d ] && echo yes)" "yes"
check "arithmetic" "$((6 * 7))" "42"

echo
if [ "$fails" -eq 0 ]; then echo "DISTRO ALL PASSED"; else echo "DISTRO $fails FAILED"; fi
exit "$fails"
