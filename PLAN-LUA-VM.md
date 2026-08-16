# Should the Lua be a real VM? Measured.

`oslo-lua` is a tree walker over `full_moon`'s AST. `docs/features/lua-interpreter.md` records why —
one Rust core behind two front ends, and no C anywhere near a static musl build — and it records
what that costs. The question is whether the trade still holds.

**It does not.** On this branch the reference VM is compiled into the binary behind a `lua-vm`
feature, and every number below comes from running the same scripts through the same oslo binary
with `OSLO_LUA_VM` set or unset. Both engines are present at once, which is what makes the
comparison fair and what makes the branch a spike rather than a switch.

## Speed

Identical scripts, best of three, `target/release/oslo` at fat LTO:

| | oslo-lua | mlua | |
|---|---|---|---|
| 1,000,000 loop iterations | 0.281 s | **0.003 s** | 94× |
| 1,000,000 table stores | 0.444 s | **0.011 s** | 40× |
| 200,000 function calls | 0.093 s | **0.004 s** | 23× |
| 100 processes, empty chunk | 0.61 s | **0.21 s** | 3× |

The first three are the same benchmarks the current documentation quotes, so they can be compared
against its recorded 0.218–0.246 s, 0.334–0.337 s and 0.072–0.078 s directly. The last one matters
most for a shell: it is startup, and the VM is three times quicker to stand up than the tree walker
is to parse and walk nothing.

## Size — the argument that inverts

The spike's binary grows, because it carries **both** engines:

```
musl static, without the VM   5,815,552 bytes
musl static, with the VM      6,090,512 bytes      +274,960
```

But a switch would delete the tree walker and its parser. Measured head to head — minimal binaries,
identical profile (fat LTO, `opt-level = "s"`, stripped), against a bare Rust floor of 292,360 bytes:

| engine | binary | engine alone |
|---|---|---|
| `oslo-lua` + `full_moon` | 923,328 | **630,968** |
| `mlua`, vendored Lua 5.4 | 608,088 | **315,728** |

**The reference C interpreter is half the size of the Rust tree walker plus its parser.** A completed
switch should land near 5,500,000 bytes — roughly 308 KB *smaller* than today, not larger. That is an
estimate from the two figures above, not a measurement of a finished port.

It also deletes `vendor/full_moon` (14,290 lines plus 926 in its derive crate) and
`crates/oslo-lua` (5,340 lines) from the tree, and with them the MPL-2.0 licence that `full_moon`
carries — `mlua` is MIT, as Lua itself is.

## The musl objection, tested

This is the reason the project moved off `mlua`, and it is worth being exact about. Vendored `mlua`
compiles Lua from C, so a musl build needs a C compiler that targets musl. Without one it fails at
link time with glibc symbols the musl libc does not have:

```
undefined reference to `__memcpy_chk'
undefined reference to `__vfprintf_chk'
undefined reference to `fopen64'
```

With one, it works. Building with `CC_x86_64_unknown_linux_musl` pointed at a musl gcc produced a
**statically linked** binary that runs the benchmarks:

```
$ file target/x86_64-unknown-linux-musl/release/oslo
ELF 64-bit LSB pie executable, x86-64, static-pie linked

$ OSLO_LUA_VM=1 ./oslo loop.lua
loop_1M 0.005
```

And the release pipeline already installs the toolchain: `.github/workflows/release.yml` runs
`apt-get install musl-tools` and checks `musl-gcc --version`. The change it needs is one environment
variable, not a new dependency on the build host.

## Capability

Nine things the documentation lists as out of reach, run through both engines in one binary:

| | oslo-lua | mlua |
|---|---|---|
| coroutines | no | **yes** |
| `goto` / labels | no | **yes** |
| `utf8` library | no | **yes** |
| `string.char(255)` is one byte | no — three | **yes** |
| `io.open` | no | **yes** |
| `string.format("%g", 1/3)` | no — 16 digits | **yes** |
| `<const>` enforced | no — assignment accepted | **yes** |
| 5,000 nested calls | no — 200 is the ceiling | **yes** |
| weak tables, collected cycles | no — leaks | **yes** |

Nine out of nine against zero out of nine. Two of these are correctness rather than convenience: a
`<const>` that accepts assignment and a string that turns `\255` into three bytes are cases where a
script runs and is quietly wrong.

## What this branch is not

It is a spike. `oslo_luavm::run` evaluates a chunk in a bare VM — the shell's own API is **not** bound
to it. That surface is the real cost of the switch:

- **130 registered callables** across `crates/oslo-runtime/src/lua/api/`, 11,562 lines
- **`oslo_lua::` types used in 70 files** — `Value`, `Table`, `LuaError`, `Interp`
- every one of them re-expressed against `mlua`'s `UserData`, `Function` and `Value`

That is the work, and none of it is measured here.

## What is actually lost

Not "control" — the VM has more of it. Two things:

1. **One Rust core, two front ends.** Today a builtin is written once and reached by both `ls -la`
   and `sh.ls("-la")` because Lua values *are* Rust values. Behind a VM they become a boundary to
   marshal across. `mlua` can hold Rust types as userdata, so this is a design problem rather than a
   wall — but it is the thing the current architecture exists to provide, and it would have to be
   rebuilt rather than kept.
2. **The build stops being pure Rust.** `cargo build` on a musl target now needs `musl-gcc` present.
   CI has it; a contributor on a fresh machine would have to install it.

## Recommendation

Worth it, on the numbers. The three arguments the current design rests on — size, no C, and control
— come out as: **half the size**, C that the release pipeline already has a compiler for, and
strictly more control. The speed is 20–90× and startup is 3×.

The cost is real and it is the binding port, not the engine. Do it as a port of
`crates/oslo-runtime/src/lua/api/` module by module against a `Lua` handle, keeping `oslo-lua`
compiled until the last module moves, and delete `full_moon` and the tree walker together at the end.

## Reproducing

```sh
cargo build --release --all-features --features lua-vm
cd bench && OSLO_LUA_VM=1 ../target/release/oslo loop.lua   # the VM
             ../target/release/oslo loop.lua                # the tree walker

# musl, with a C compiler that targets it
CC_x86_64_unknown_linux_musl=musl-gcc \
RUSTFLAGS="-C target-feature=+crt-static" \
cargo build --release --target x86_64-unknown-linux-musl --features lua-vm
```
