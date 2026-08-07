#!/usr/bin/env bash
# Boot a real Arch Linux userland with oslo as /bin/sh.
#
# The companion to `alpine-distro-vm.sh`, and the harder of the two. Alpine's `/bin/sh` is busybox
# ash; **Arch's is bash**. Every `#!/bin/sh` script Arch ships was therefore written against bash
# and may legitimately use bashisms, so standing in for it is a bash-compatibility test rather than
# a POSIX one — which is exactly why it is worth running.
#
# Arch also brings glibc and systemd where Alpine brings musl and OpenRC, so between the two the
# shell is exercised against both halves of the Linux world.
#
# The rootfs is the official bootstrap tarball, unpacked and handed to the kernel as an initramfs.
# No `pacstrap`, no root: extraction gets the files, and the ownership it cannot set is all
# root:root anyway on a VM that runs as root. The kernel is Alpine's `virt` build, because the
# kernel ABI is stable and a glibc userland does not care which kernel it was compiled beside.
#
# Usage: scripts/arch-vm.sh [--shell]
#   (no args)  build, boot, run the suite, exit non-zero on failure
#   --shell    boot to an interactive oslo instead of running the suite
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
work=${OSLO_VM_WORK:-/tmp/oslo-arch-vm}
mirror=${OSLO_ARCH_MIRROR:-https://geo.mirror.pkgbuild.com/iso/latest}
alpine=${OSLO_ALPINE_MIRROR:-https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/x86_64}

say() { printf '\n== %s\n' "$*"; }

interactive=false
[ "${1:-}" = "--shell" ] && interactive=true

mkdir -p "$work"
cd "$work"

say "building the static binary"
# Static musl, which is what makes one binary run on Alpine *and* Arch: nothing is resolved at
# load time, so glibc's absence or presence is not its business.
(cd "$here" && make build >/dev/null)
binary=$here/target/x86_64-unknown-linux-musl/release/oslo
[ -x "$binary" ] || { echo "no static binary at $binary" >&2; exit 1; }

say "fetching Arch"
[ -f vmlinuz-virt ] || curl -sfLO "$alpine/netboot/vmlinuz-virt"
[ -f archlinux-bootstrap-x86_64.tar.zst ] ||
    curl -sfLO "$mirror/archlinux-bootstrap-x86_64.tar.zst"

if [ ! -d root.x86_64 ]; then
    say "unpacking (a few hundred megabytes; the ownership warnings are expected)"
    # `|| true`: unpacking as an ordinary user cannot create device nodes or chown, and says so
    # about a few hundred files. None of it matters for a boot test.
    tar --zstd -xf archlinux-bootstrap-x86_64.tar.zst 2>/dev/null || true
    # A few files arrive setuid-and-unreadable (`dbus-daemon-launch-helper` is 4750 root:dbus),
    # which stops an ordinary user copying the tree at all. The VM runs as root and reinstates
    # nothing, so readable-by-owner is the right trade for being able to build this without sudo.
    chmod -R u+rwX root.x86_64 2>/dev/null || true
fi

say "installing oslo as /bin/sh"
rootfs=$work/rootfs
# Made writable before removing: a previous run's tree carries Arch's own directory modes, and
# some of them do not let their owner delete what is inside.
[ -d "$rootfs" ] && chmod -R u+rwX "$rootfs" 2>/dev/null
rm -rf "$rootfs"
cp -a root.x86_64 "$rootfs"
install -Dm755 "$binary" "$rootfs/usr/bin/oslo"
# **The point of the whole exercise.** `/bin` is a symlink to `usr/bin` on Arch, so there is one
# real file to replace and both paths follow it.
ln -sf oslo "$rootfs/usr/bin/sh"
install -Dm755 "$here/scripts/arch-suite.sh" "$rootfs/arch-suite.sh"

# The init the kernel starts. Written as shell and run by oslo, so oslo is PID 1 as well as
# `/bin/sh` — if it cannot bring the system this far, nothing after it is worth measuring.
cat > "$rootfs/init" <<'INIT'
#!/bin/sh
mount -t proc none /proc 2>/dev/null
mount -t sysfs none /sys 2>/dev/null
mount -t devtmpfs none /dev 2>/dev/null
mount -t tmpfs none /tmp 2>/dev/null
mount -t tmpfs none /run 2>/dev/null
ln -sf /proc/self/fd /dev/fd 2>/dev/null
export PATH=/usr/local/sbin:/usr/local/bin:/usr/bin:/usr/sbin
export HOME=/root TERM=linux

echo "ARCH-BOOT: $(uname -sr)"
/arch-suite.sh
echo "ARCH-SUITE-DONE:$?"
poweroff -f 2>/dev/null || { echo o > /proc/sysrq-trigger; sleep 5; }
INIT
chmod +x "$rootfs/init"

if $interactive; then
    # Drop to a prompt instead of running the suite, for looking around by hand.
    sed -i 's|^/arch-suite.sh$|exec /bin/sh -i|; /ARCH-SUITE-DONE/d; /poweroff/d' "$rootfs/init"
fi

say "packing the initramfs (the whole userland goes in RAM)"
( cd "$rootfs" && find . -print0 | cpio --null -o --format=newc 2>/dev/null ) | gzip -1 > arch-initramfs.gz
ls -lh arch-initramfs.gz | awk '{print "  " $5}'

qemu_args=(
    -kernel vmlinuz-virt
    -initrd arch-initramfs.gz
    # 3 GB: the rootfs is half a gigabyte and it is *in* the ramdisk, twice over while it unpacks.
    -m 3G
    -nographic
    -no-reboot
    -append "console=ttyS0 quiet"
)

say "booting"
if $interactive; then
    exec qemu-system-x86_64 "${qemu_args[@]}"
fi

log=$work/arch-boot.log
qemu-system-x86_64 "${qemu_args[@]}" </dev/null >"$log" 2>&1 &
qemu_pid=$!
deadline=$((SECONDS + 600))
while kill -0 "$qemu_pid" 2>/dev/null; do
    grep -aqE 'ARCH-SUITE-EXIT:[0-9]+' "$log" && break
    if [ "$SECONDS" -ge "$deadline" ]; then
        break
    fi
    sleep 1
done
if kill -0 "$qemu_pid" 2>/dev/null; then
    kill "$qemu_pid" 2>/dev/null || true
fi
wait "$qemu_pid" 2>/dev/null || true

say "result"
sed -n '/ARCH-BOOT/,/ARCH-SUITE-EXIT/p' "$log" | sed 's/\r$//'
code=$(grep -aoE 'ARCH-SUITE-EXIT:[0-9]+' "$log" | tail -1 | cut -d: -f2)
if [ -z "$code" ]; then
    echo "the suite never reported: the system did not come up far enough" >&2
    tail -25 "$log" | sed 's/\r$//' >&2
    exit 1
fi
exit "$code"
