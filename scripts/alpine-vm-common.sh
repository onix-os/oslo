# Shared by the two Alpine VMs: `alpine-vm.sh` (a minirootfs, hermetic and quick) and
# `alpine-distro-vm.sh` (a real Alpine userland running OpenRC). Sourced, never executed.
#
# What belongs here is only what both need and neither owns: producing the static musl binary,
# fetching Alpine's pieces, and starting qemu. Everything about *what is tested* stays in the
# script that tests it.

mirror=https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/x86_64
packages=https://dl-cdn.alpinelinux.org/alpine/latest-stable/main/x86_64
target=x86_64-unknown-linux-musl

say() { printf '\n== %s\n' "$*"; }

# Build the release binary Alpine can actually run.
#
# Alpine is musl, so this is not an optimisation — a glibc build cannot execute there. The linker
# is deliberately left alone: pointing it at musl-gcc yields a binary that records the *build
# host's* loader path and dies on any other machine. See README's "Installing" section.
#
# No C toolchain is needed. It used to be, for mlua's vendored Lua; oslo evaluates Lua in Rust now,
# and nothing left in the dependency tree compiles C.
#
# Echoes the path to the binary.
build_static_oslo() {
    # The Rust side needs a musl `std`, and the toolchain on `$PATH` may not be the one that has
    # it — a nixpkgs `rustc` ships only the targets nixpkgs built it with, while the rustup
    # toolchain beside it may have had `rustup target add` run against it. Without this check the
    # failure is thirty lines of "can't find crate for `core`", naming neither cause nor fix.
    if [ ! -d "$(rustc --print target-libdir --target "$target" 2>/dev/null)" ]; then
        echo "rustc ($(command -v rustc)) has no std for $target." >&2
        echo "try: rustup target add $target   (rustup's shims must come first on PATH)" >&2
        exit 1
    fi

    RUSTFLAGS="-C target-feature=+crt-static" \
        cargo build --release --locked --target "$target" --bin oslo \
        --manifest-path "$here/Cargo.toml" >&2

    local binary="$here/target/$target/release/oslo"
    [ -x "$binary" ] || binary="${CARGO_TARGET_DIR:-$here/target}/$target/release/oslo"
    if readelf -l "$binary" | grep -q 'program interpreter'; then
        echo "the binary is not static; it cannot run on Alpine" >&2
        exit 1
    fi
    echo "$binary"
}

# Alpine's own `virt` kernel, cached. Not the host's: that one is usually root-only, and the virt
# flavour has virtio and ext4 built in, so no module loading is needed.
fetch_kernel() {
    if [ ! -f vmlinuz-virt ]; then
        say "fetching the Alpine virt kernel" >&2
        curl -sfLO "$mirror/netboot/vmlinuz-virt"
    fi
}

# The minirootfs tarball, cached. Echoes its filename.
fetch_minirootfs() {
    local tarball
    tarball=$(ls alpine-minirootfs-*-x86_64.tar.gz 2>/dev/null | head -1 || true)
    if [ -z "$tarball" ]; then
        say "fetching the Alpine minirootfs" >&2
        tarball=$(curl -sf "$mirror/latest-releases.yaml" |
            grep -m1 -oE 'alpine-minirootfs-[0-9.]+-x86_64\.tar\.gz')
        curl -sfLO "$mirror/$tarball"
    fi
    echo "$tarball"
}

# Pack a directory into a gzipped cpio initramfs.
#
# The initramfs *is* the root filesystem in both VMs: no disk, no bootloader, no partitioning —
# the kernel unpacks it and runs `/init`. That is the shortest path to running as PID 1, and it is
# how a real initramfs works.
pack_initramfs() {
    (cd "$1" && find . | cpio -o -H newc --quiet | gzip -9) >"$2"
    say "initramfs: $(du -h "$2" | cut -f1)" >&2
}

# The qemu command line both VMs boot with. `rdinit` rather than `init`: there is no switch_root
# here, the initramfs is the system.
qemu_args_for() {
    qemu_args=(
        -kernel vmlinuz-virt
        -initrd "$1"
        -m "${2:-512M}"
        -no-reboot
        -nographic
        -append "console=ttyS0 rdinit=/init panic=1 loglevel=3"
    )
    [ -w /dev/kvm ] && qemu_args+=(-enable-kvm)
    return 0
}
