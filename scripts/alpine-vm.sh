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
# Usage: scripts/alpine-vm.sh [--shell]
#   (no args)  build, boot, run the suite, print the result, exit non-zero on failure
#   --shell    boot to an interactive oslo prompt instead, for poking around by hand
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
work=${OSLO_VM_WORK:-/tmp/oslo-alpine-vm}
mirror=https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/x86_64
interactive=false
[ "${1:-}" = "--shell" ] && interactive=true

mkdir -p "$work"
cd "$work"

say() { printf '\n== %s\n' "$*"; }

# ---------------------------------------------------------------- the static binary
say "building the static musl binary"
target=x86_64-unknown-linux-musl
# Alpine is musl, so this is not an optimisation — a glibc build cannot execute there.
# The linker is deliberately left alone: pointing it at musl-gcc yields a binary that records the
# *build host's* loader path and dies on any other machine. Only the C compiler for mlua's
# vendored Lua is set. See README's "Installing" section.
: "${CC_x86_64_unknown_linux_musl:=musl-gcc}"
export CC_x86_64_unknown_linux_musl
RUSTFLAGS="-C target-feature=+crt-static" \
    cargo build --release --locked --target "$target" --bin oslo --manifest-path "$here/Cargo.toml"
binary="$here/target/$target/release/oslo"
[ -x "$binary" ] || binary="${CARGO_TARGET_DIR:-$here/target}/$target/release/oslo"

if readelf -l "$binary" | grep -q 'program interpreter'; then
    echo "the binary is not static; it cannot run on Alpine" >&2
    exit 1
fi

# ---------------------------------------------------------------- Alpine pieces, cached
rootfs_tar=$(ls alpine-minirootfs-*-x86_64.tar.gz 2>/dev/null | head -1 || true)
if [ -z "$rootfs_tar" ]; then
    say "fetching the Alpine minirootfs"
    rootfs_tar=$(curl -sf "$mirror/latest-releases.yaml" |
        grep -m1 -oE 'alpine-minirootfs-[0-9.]+-x86_64\.tar\.gz')
    curl -sfLO "$mirror/$rootfs_tar"
fi
if [ ! -f vmlinuz-virt ]; then
    say "fetching the Alpine virt kernel"
    curl -sfLO "$mirror/netboot/vmlinuz-virt"
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
chmod +x root/vm-suite.sh

if $interactive; then
    cat >root/init <<'INIT'
#!/bin/oslo
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
ln -sf /proc/self/fd /dev/fd
echo
echo "oslo is PID 1 and /bin/sh here. 'poweroff -f' to leave."
exec /bin/oslo -i
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
# `<(cmd)` names `/dev/fd/N`, which is a symlink to /proc/self/fd that a minirootfs does not ship.
# Creating it is part of bringing a system up; without it process substitution cannot work in any
# shell, bash included.
ln -sf /proc/self/fd /dev/fd
/vm-suite.sh
echo "VM-SUITE-EXIT:$?"
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

(cd root && find . | cpio -o -H newc --quiet | gzip -9) >initramfs.gz
say "initramfs: $(du -h initramfs.gz | cut -f1)"

# ---------------------------------------------------------------- boot
say "booting"
qemu_args=(
    -kernel vmlinuz-virt
    -initrd initramfs.gz
    -m 512M
    -no-reboot
    -nographic
    # `rdinit` rather than `init`: there is no switch_root here, the initramfs *is* the system.
    -append "console=ttyS0 rdinit=/init panic=1 loglevel=3"
)
command -v kvm >/dev/null 2>&1 && qemu_args+=(-enable-kvm)
[ -w /dev/kvm ] && qemu_args+=(-enable-kvm)

if $interactive; then
    exec qemu-system-x86_64 "${qemu_args[@]}"
fi

log=$work/boot.log
timeout 300 qemu-system-x86_64 "${qemu_args[@]}" 2>&1 | tee "$log" || true

say "result"
sed -n '/VM-SUITE-BEGIN/,/VM-SUITE-EXIT/p' "$log" | sed 's/\r$//'
code=$(grep -oE 'VM-SUITE-EXIT:[0-9]+' "$log" | tail -1 | cut -d: -f2)
if [ -z "$code" ]; then
    echo "the suite never reported: oslo did not get far enough as PID 1" >&2
    exit 1
fi
exit "$code"
