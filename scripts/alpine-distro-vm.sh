#!/usr/bin/env bash
# Boot a *real* Alpine userland with oslo as /bin/sh, and let OpenRC bring the system up.
#
# `alpine-vm.sh` proves oslo can be PID 1 in an empty room. This proves something else, and it is
# the thing the project is actually for: that a distro's own init system, written in POSIX shell
# by people who have never heard of oslo, runs on it unmodified.
#
# The difference is not cosmetic. The minirootfs suite reported "parsed 2 of the image's own
# scripts" — there were only two. Alpine's base with OpenRC has hundreds: every `/etc/init.d/*`
# service, the `/lib/rc/sh/*` runtime that sources them, and `alpine-conf`'s `setup-*` tools,
# which are among the densest POSIX shell any distro ships.
#
# Built by layering packages onto the cached minirootfs rather than bootstrapping with `apk`,
# because `apk` wants to chown and mknod and this runs as an ordinary user. Extraction gets the
# files, which is all a boot test needs; the ownership it skips is all root:root anyway, and the
# VM runs as root.
#
# Usage: scripts/alpine-distro-vm.sh [--shell]
#   (no args)  build, boot, run OpenRC, run the suite, exit non-zero on failure
#   --shell    boot to an interactive oslo *after* OpenRC has brought the system up
set -euo pipefail

here=$(cd -- "$(dirname -- "$0")/.." && pwd)
work=${OSLO_VM_WORK:-/tmp/oslo-alpine-vm}
. "$here/scripts/alpine-vm-common.sh"

interactive=false
[ "${1:-}" = "--shell" ] && interactive=true

mkdir -p "$work"
cd "$work"

say "building the static musl binary"
binary=$(build_static_oslo)

say "fetching Alpine"
fetch_kernel
rootfs_tar=$(fetch_minirootfs)

# The packages that turn a minirootfs into a system that boots.
#
# `openrc` is the init system. `busybox-openrc` and `busybox-mdev-openrc` are the `/etc/init.d`
# services for the daemons busybox provides — they are the shell scripts under test. `alpine-conf`
# is included for its `setup-*` scripts: nothing here runs them, but they are several thousand
# lines of real POSIX shell for the parse sweep to chew on, which is exactly the corpus a
# hand-written test suite cannot produce.
distro_packages=(
    openrc
    busybox-openrc
    busybox-mdev-openrc
    alpine-conf
    alpine-baselayout
    libcap2
)

mkdir -p pkgs
for pkg in "${distro_packages[@]}"; do
    if ! ls "pkgs/$pkg"-[0-9]*.apk >/dev/null 2>&1; then
        say "fetching $pkg"
        # The index is HTML; the newest build of a package is the last matching href.
        #
        # Anchored on `href="` and on `-[0-9]`, and both anchors are load-bearing. Without the
        # first, `openrc` matches inside `acct-openrc-6.6.4-r2.apk` and yields a filename that
        # does not exist; without the second, `busybox` matches `busybox-openrc`.
        file=$(curl -sf "$packages/" |
            grep -oE "href=\"${pkg}-[0-9][^\"]*\.apk\"" |
            sed 's/^href="//; s/"$//' | sort -V | tail -1)
        if [ -z "$file" ]; then
            echo "no package found for $pkg" >&2
            exit 1
        fi
        curl -sfL -o "pkgs/$file" "$packages/$file"
    fi
done

say "assembling the root filesystem"
rm -rf distro && mkdir distro
tar -xzf "$rootfs_tar" -C distro
for pkg in "${distro_packages[@]}"; do
    file=$(ls "pkgs/$pkg"-[0-9]*.apk | sort -V | tail -1)
    # An .apk is a gzipped tar with signature and metadata members whose names begin with a dot.
    # `--no-same-owner` because this is not root, and `|| true` because tar reports the metadata
    # members it cannot map to paths.
    tar -xzf "$file" -C distro --no-same-owner \
        --exclude='.SIGN.*' --exclude='.PKGINFO' --exclude='.pre-install' \
        --exclude='.post-install' --exclude='.trigger' 2>/dev/null || true
done

mkdir -p distro/proc distro/sys distro/dev distro/run distro/tmp distro/var/log

install -m755 "$binary" distro/bin/oslo
# oslo *is* the system shell. Every `/etc/init.d` service, every `/lib/rc/sh` helper and every
# `#!/bin/sh` in the image runs under it from here.
ln -sf oslo distro/bin/sh
grep -q '^/bin/oslo$' distro/etc/shells 2>/dev/null || echo /bin/oslo >>distro/etc/shells

cp "$here/scripts/alpine-distro-suite.sh" distro/distro-suite.sh
chmod +x distro/distro-suite.sh

# OpenRC refuses to run without these; a real Alpine image ships them and the minirootfs does not.
mkdir -p distro/run/openrc distro/etc/runlevels/sysinit distro/etc/runlevels/boot \
    distro/etc/runlevels/default
: >distro/run/openrc/softlevel

# Enable a handful of services, chosen because each is a *shell script* that has to run rather
# than a daemon that has to work: they source `/lib/rc/sh/functions.sh`, parse their own options,
# and use the shell hard enough to be a real test of it.
for svc in hostname sysctl bootmisc syslog; do
    [ -f "distro/etc/init.d/$svc" ] && ln -sf "/etc/init.d/$svc" "distro/etc/runlevels/boot/$svc"
done
[ -f distro/etc/init.d/devfs ] && ln -sf /etc/init.d/devfs distro/etc/runlevels/sysinit/devfs

if $interactive; then
    cat >distro/init <<'INIT'
#!/bin/oslo
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null
ln -sf /proc/self/fd /dev/fd
openrc sysinit
openrc boot
openrc default
echo
echo "oslo is /bin/sh and OpenRC has run. 'poweroff -f' to leave."
exec setsid -c /bin/oslo -i </dev/ttyS0 >/dev/ttyS0 2>&1
INIT
else
    cat >distro/init <<'INIT'
#!/bin/oslo
# PID 1 in a system that has an init system. Unlike the minirootfs VM, the point here is not that
# oslo *is* init — it is that OpenRC's own shell runs on it.
export PATH=/usr/bin:/usr/sbin:/bin:/sbin
mount -t proc proc /proc 2>/dev/null
mount -t sysfs sys /sys 2>/dev/null
mount -t devtmpfs dev /dev 2>/dev/null
mount -t tmpfs tmpfs /run 2>/dev/null
mkdir -p /run/openrc
: >/run/openrc/softlevel
ln -sf /proc/self/fd /dev/fd

echo "OPENRC-BEGIN"
openrc sysinit
echo "OPENRC-SYSINIT:$?"
openrc boot
echo "OPENRC-BOOT:$?"
openrc default
echo "OPENRC-DEFAULT:$?"

/distro-suite.sh
echo "DISTRO-SUITE-EXIT:$?"
poweroff -f
INIT
fi
chmod +x distro/init

pack_initramfs distro distro-initramfs.gz

say "booting"
qemu_args_for distro-initramfs.gz 768M

if $interactive; then
    exec qemu-system-x86_64 "${qemu_args[@]}"
fi

log=$work/distro-boot.log
timeout 420 qemu-system-x86_64 "${qemu_args[@]}" 2>&1 | tee "$log" >/dev/null || true

say "result"
sed -n '/OPENRC-BEGIN/,/DISTRO-SUITE-EXIT/p' "$log" | sed 's/\r$//'
code=$(grep -aoE 'DISTRO-SUITE-EXIT:[0-9]+' "$log" | tail -1 | cut -d: -f2)
if [ -z "$code" ]; then
    echo "the suite never reported: the system did not come up far enough" >&2
    tail -20 "$log" | sed 's/\r$//' >&2
    exit 1
fi
exit "$code"
