#!/usr/bin/env bash
# Boot oslo as an Alpine VM's /bin/sh *and* as its PID 1, and run a test suite in there.
#
# This exists because two things cannot be tested from a checkout, and both are the point of the
# project:
#
#   * **PID 1.** An initramfs `/bin/sh` is init. Whether it reaps orphans, survives signals and
#     brings a system up is not observable in a test harness that is itself a child process.
#   * **A foreign userland.** Alpine is musl and busybox, not glibc and coreutils. It is the
#     closest thing to the distro oslo is meant to ship in, and every utility here is a different
#     implementation from the one the differential corpus was written against.
#
# Design, and why each piece is the way it is:
#
#   * **The rootfs is the initramfs.** No disk, no bootloader, no partitioning — the kernel unpacks
#     it and runs `/init`, which *is* oslo. That is the shortest path to running as PID 1, and it
#     is how a real initramfs works.
#   * **Alpine's own `virt` kernel**, not the host's. The host kernel is usually root-only, and the
#     virt flavour has virtio and ext4 built in, so no module loading is needed.
#   * **The binary is the static musl release.** Alpine has no glibc, so a dynamically linked build
#     would not start at all. That is a feature of this test: it proves the release artifact runs
#     on a system that shares nothing with the build host.
#
# Usage: scripts/alpine-vm.sh [--shell | --console]
#   (no args)  build, boot, run both suites, print the result, exit non-zero on failure
#   --shell    boot to an interactive oslo prompt instead, for poking around by hand
#   --console  boot to an interactive oslo and *type at it from the host*: the only way to test
#              the characters, Ctrl-C and Ctrl-Z, that only a terminal driver can produce
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
work=${OSLO_VM_WORK:-/tmp/oslo-alpine-vm}
. "$here/scripts/alpine-vm-common.sh"
mode=suite
case "${1:-}" in
--shell) mode=shell ;;
--console) mode=console ;;
"") ;;
*)
    echo "usage: $0 [--shell | --console]" >&2
    exit 2
    ;;
esac
interactive=false
[ "$mode" = shell ] && interactive=true

mkdir -p "$work"
cd "$work"

say "building the static musl binary"
binary=$(build_static_oslo)

# ---------------------------------------------------------------- Alpine pieces, cached
rootfs_tar=$(fetch_minirootfs)
fetch_kernel
# modernish is the external conformance oracle: a POSIX-shell library whose whole install ritual
# is a battery of probes for known shell bugs, each with a name. It is a *second opinion* — its
# expectations were written against a dozen real shells, not against bash, and not by us.
#
# Fetched on the host and baked into the image because the VM has no network: the initramfs is a
# bare minirootfs with no interface configured, and giving it one would mean a DHCP client and a
# resolver in a test that is otherwise hermetic.
if [ ! -f modernish.tar.gz ]; then
    say "fetching modernish (the conformance oracle)"
    curl -sfL -o modernish.tar.gz \
        https://github.com/modernish/modernish/archive/refs/heads/master.tar.gz
fi

# ---------------------------------------------------------------- assemble the root
say "assembling the root filesystem"
rm -rf root && mkdir root
tar -xzf "$rootfs_tar" -C root
mkdir -p root/proc root/sys root/dev root/tmp

install -m755 "$binary" root/bin/oslo

# oslo *is* the system shell. Alpine's /bin/sh is busybox ash, so replacing it means every script
# in the image — busybox's own included — runs under oslo.
ln -sf oslo root/bin/sh
grep -q '^/bin/oslo$' root/etc/shells 2>/dev/null || echo /bin/oslo >>root/etc/shells

# The suite. Written in shell on purpose: it is the thing under test, and a Lua suite would only
# prove the Lua half. `/init` runs as PID 1.
cp "$here/scripts/alpine-vm-suite.sh" root/vm-suite.sh
cp "$here/scripts/alpine-vm-jobs.sh" root/vm-jobs.sh
chmod +x root/vm-suite.sh root/vm-jobs.sh

mkdir -p root/opt
tar -xzf modernish.tar.gz -C root/opt
mv root/opt/modernish-master root/opt/modernish

if [ "$mode" = console ]; then
    cat >root/init <<'INIT'
#!/bin/oslo
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null
ln -sf /proc/self/fd /dev/fd
# `setsid -c` on /dev/ttyS0, not `exec /bin/oslo -i` on the console: only a *controlling terminal*
# turns a typed ^C into a SIGINT for the foreground process group, and /dev/console can never be
# one. This is what makes the characters the host types below reach the tty driver at all.
# Set the control characters explicitly, as a getty would, so the test measures the shell rather
# than whatever this console happened to boot with. (`stty -a` reports `intr = ^C; susp = ^Z` on
# ttyS0 even without this, so it is belt-and-braces rather than the fix it was first written as.)
stty intr ^C susp ^Z </dev/ttyS0
echo "CONSOLE-READY"
# **Not `exec`**, and this is the whole reason ^Z works here. The kernel discards SIGTSTP, SIGTTIN
# and SIGTTOU sent to an *orphaned* process group, and `will_become_orphaned_pgrp()` counts a job
# as orphaned when its parent is PID 1 (`is_global_init(p->real_parent)`). With `exec`, the
# interactive shell *was* PID 1, so every job it started was in such a group and ^Z did nothing —
# for a full afternoon that read as a shell bug. ^C was unaffected because SIGINT is not a stop
# signal, and that asymmetry was the clue.
#
# A real system never has this shape: getty, login or sshd always sits between init and a login
# shell. Keeping the shell a child of init is what makes this VM resemble one.
setsid -c /bin/oslo -i </dev/ttyS0 >/dev/ttyS0 2>&1
poweroff -f
INIT
elif $interactive; then
    cat >root/init <<'INIT'
#!/bin/oslo
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null
ln -sf /proc/self/fd /dev/fd
echo
echo "oslo is PID 1 and /bin/sh here. 'poweroff -f' to leave."
exec setsid -c /bin/oslo -i </dev/ttyS0 >/dev/ttyS0 2>&1
INIT
else
    cat >root/init <<'INIT'
#!/bin/oslo
# PID 1. /proc is mounted here rather than by the kernel so that the "no /proc" case above it
# stays observable: the suite checks the shell works before this runs.
# Nothing sets PATH in an initramfs — there is no login and no profile — so every busybox tool
# is "command not found" until this line. It is the first thing a real init does too.
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
# devtmpfs so that /dev/ttyS0 exists as a real device node. The job-control suite needs to *open*
# the console as a controlling terminal, which /dev/console cannot be made to be.
mount -t devtmpfs dev /dev 2>/dev/null
# `<(cmd)` names `/dev/fd/N`, which is a symlink to /proc/self/fd that a minirootfs does not ship.
# Creating it is part of bringing a system up; without it process substitution cannot work in any
# shell, bash included.
ln -sf /proc/self/fd /dev/fd
/vm-suite.sh
echo "VM-SUITE-EXIT:$?"

# Job control needs a *controlling terminal*, which init does not have: the kernel hands PID 1
# /dev/console, and console is deliberately not claimable as a ctty. `setsid -c` starts a new
# session with /dev/ttyS0 as its controlling terminal, which is the only way to reach `tcsetpgrp`,
# `fg`, `bg` and the terminal's foreground process group from inside the VM.
timeout 120 setsid -c /bin/sh /vm-jobs.sh </dev/ttyS0 >/dev/ttyS0 2>&1
echo "VM-JOBS-EXIT:$?"
# Reaping happens at a command boundary, and this is one: a shell as init cannot reap while it is
# blocked in a foreground command, so the check belongs here rather than inside the suite.
sh -c '(sleep 0.1 &) ; exit 0'
sleep 1
:
echo "ORPHANS-AT-BOUNDARY:$(ps -o stat= | grep -c "^Z")"
poweroff -f
INIT
fi
chmod +x root/init

pack_initramfs root initramfs.gz

# ---------------------------------------------------------------- boot
say "booting"
qemu_args_for initramfs.gz

if $interactive; then
    exec qemu-system-x86_64 "${qemu_args[@]}"
fi

if [ "$mode" = console ]; then
    # ---------------------------------------------------------------- typing at it
    #
    # The last part of job control that nothing else can reach: the characters. A ^C is not a
    # signal a test can send — it is a byte in the terminal's *input* queue that the line
    # discipline turns into a SIGINT for whatever process group the terminal considers foreground.
    # Only the far end of the line can put it there, and with `-nographic` that far end is this
    # script's stdin to qemu.
    #
    # The fifo is held open on fd 9 for the whole run. Without that, qemu sees EOF the moment the
    # first `printf` finishes and the guest's console read returns nothing for ever after.
    console_in=$work/console.in
    log=$work/console.log
    rm -f "$console_in" "$log"
    mkfifo "$console_in"
    timeout 240 qemu-system-x86_64 "${qemu_args[@]}" <"$console_in" >"$log" 2>&1 &
    qemu_pid=$!
    exec 9>"$console_in"

    # Wait for text to appear in the log, or give up. Polling a file rather than reading a pipe
    # because the same bytes have to stay readable afterwards for the assertions.
    await() {
        local want=$1 limit=${2:-30} waited=0
        while [ "$waited" -lt "$limit" ]; do
            grep -q "$want" "$log" 2>/dev/null && return 0
            sleep 1
            waited=$((waited + 1))
        done
        return 1
    }
    send() { printf '%s' "$1" >&9; }

    if ! await CONSOLE-READY 90; then
        echo "the VM never reached a console prompt" >&2
        kill "$qemu_pid" 2>/dev/null || true
        sed -n '$p' "$log" >&2
        exit 1
    fi
    sleep 2

    # Markers are spelled with a `''` in what is *typed* so that the terminal's echo of the typed
    # line cannot be mistaken for the command's output — the shell prints `MARKER1`, the echo shows
    # `MARK''ER1`. Without that every `await` matches its own keystrokes and proves nothing.
    send $'echo MARK\'\'ER1\n'
    await MARKER1 20 || {
        echo "the console never answered" >&2
        exit 1
    }

    say "typing ^C at a running foreground job"
    send $'sleep 60\n'
    sleep 3
    send $'\003'
    sleep 2
    send $'echo INT\'\'STATUS=$?\n'
    await 'INTSTATUS=130' 20 && echo "  ok    ^C interrupts the foreground job and reports 130" ||
        {
            echo "  FAIL  ^C did not interrupt the job"
            fail=1
        }

    say "typing ^Z, then bg and kill"
    send $'sleep 60\n'
    sleep 3
    send $'\032'
    sleep 2
    send $'echo SUSP\'\'STATUS=$?\n'
    if await 'SUSPSTATUS=148' 20; then
        echo "  ok    ^Z stops the foreground job and reports 128+SIGTSTP"
    else
        echo "  FAIL  ^Z did not stop the job"
        fail=1
    fi
    send $'jobs\n'
    await 'Stopped' 15 && echo "  ok    jobs lists it as stopped" || {
        echo "  FAIL  the stopped job is not in the job table"
        fail=1
    }
    send $'bg\n'
    sleep 2
    send $'kill %1\n'
    sleep 2
    send $'echo STILL\'\'ALIVE\n'
    await 'STILLALIVE' 20 && echo "  ok    the shell survives ^Z, bg and kill %1" || {
        echo "  FAIL  the shell did not survive the job-control sequence"
        fail=1
    }

    say "the terminal is still sane"
    send $'echo DO\'\'NE\n'
    await 'DONE' 20 && echo "  ok    the terminal still accepts input" || {
        echo "  FAIL  the terminal is wedged"
        fail=1
    }

    send $'poweroff -f\n'
    sleep 3
    exec 9>&-
    kill "$qemu_pid" 2>/dev/null || true
    wait "$qemu_pid" 2>/dev/null || true
    exit "${fail:-0}"
fi

log=$work/boot.log
timeout 300 qemu-system-x86_64 "${qemu_args[@]}" 2>&1 | tee "$log" || true

say "result"
sed -n '/VM-SUITE-BEGIN/,/VM-JOBS-EXIT/p' "$log" | sed 's/\r$//'
code=$(grep -oE 'VM-SUITE-EXIT:[0-9]+' "$log" | tail -1 | cut -d: -f2)
jobs_code=$(grep -oE 'VM-JOBS-EXIT:[0-9]+' "$log" | tail -1 | cut -d: -f2)
if [ -z "$code" ]; then
    echo "the suite never reported: oslo did not get far enough as PID 1" >&2
    exit 1
fi
if [ -z "$jobs_code" ]; then
    echo "the job-control suite never reported; it needs setsid and /dev/ttyS0" >&2
    exit 1
fi
exit $((code + jobs_code))
