-- oslo's own build, as recipes.
--
-- The `Makefile` beside this file still works and still ships; this is the same build written in
-- the language the rest of the configuration is written in, and it is the comparison the feature
-- was meant to be judged on. See `plans/PLAN_MAKE.md` and `docs/features/build-recipes.md`.
--
--   oslo make            the recipes, with what each of them says it does
--   oslo make build      the static release binary
--   oslo make verify     the whole local gate
--
-- At an oslo prompt in this directory, `make` is enough — the builtin hands the word over to the
-- program everywhere else. See `crates/oslo-shell/src/env/builtins/make.rs`.

local make = oslo.make

---------------------------------------------------------------------------- what the build is

-- The one thing `Makefile` needs a helper script for. `$(shell ...)` cannot hold a literal `#`
-- without older GNU Make mis-parsing it, so the parsing moved into `scripts/project-meta.sh`;
-- here it is two lines and no script.
local meta = oslo.run{ "./scripts/project-meta.sh", capture = true }
assert(meta.ok, "scripts/project-meta.sh: " .. (meta.err or "failed"))
local NAME, VERSION = meta.out:match("(%S+)%s+(%S+)")
assert(NAME, "PROJECT file not found or invalid")

local TARGET = os.getenv("TARGET") or "x86_64-unknown-linux-musl"
local PREFIX = os.getenv("PREFIX") or (os.getenv("HOME") .. "/.local")
local BIN = ("target/%s/release/%s"):format(TARGET, NAME)

-- oslo's own crates, and not the vendored ones. The crates under `vendor/` carry upstream test
-- modules whose dev-dependencies were never vendored, so a plain `--workspace` fails to compile
-- before it runs anything of ours.
local OURS = { "--workspace", "--exclude", "argc", "--exclude", "brush-parser",
               "--exclude", "full_moon", "--exclude", "full_moon_derive" }

-- Every `.rs` the build depends on, for the recipes that declare staleness.
local SOURCES = { "src/**/*.rs", "crates/**/*.rs", "Cargo.toml", "Cargo.lock" }

-- `cargo(...)` with lists spliced in wherever they appear.
--
-- **This exists because of a real Lua trap, and it bit this file first.** A call that expands to
-- several values is truncated to one everywhere but the last position, so writing the exclusions
-- as `cargo("clippy", ..., table.unpack(OURS), "--", "-D", "warnings")` passed `--workspace` and
-- silently dropped every `--exclude` — clippy then tried to compile the vendored crates and died
-- on their missing dev-dependencies. The same truncation is written up in
-- `docs/features/directory-environments.md` for table constructors; arguments have it too.
--
-- Splicing lists instead means the trap cannot be written down: a table is one value wherever it
-- appears, and this is what flattens it.
local function flat(...)
  local out = {}
  for _, item in ipairs({ ... }) do
    if type(item) == "table" then
      for _, word in ipairs(item) do out[#out + 1] = word end
    else
      out[#out + 1] = item
    end
  end
  return out
end

local function cargo(...)
  return sh.cargo(table.unpack(flat(...)))
end

---------------------------------------------------------------------------- building

-- `Makefile` prints its banner with `$(info …)` on every target, including `-s` and including the
-- ones whose whole output is meant to be piped. A recipe is the honest place for it: ask, and it
-- answers.
make.recipe{ name = "version", desc = "what this checkout calls itself",
             run = function() print(("%s v%s"):format(NAME, VERSION)) end }

make.recipe{
  name = "build",
  desc = "the static release binary, all features (--type minimal for none)",
  deps = { "_static-check-deps" },
  inputs = SOURCES,
  outputs = { BIN },
  stale = "content",
  params = { { "--type", desc = "minimal | full", default = "full" } },
  run = function(a)
    -- A login shell linked against a /nix/store glibc stops existing the day
    -- `nix-collect-garbage` runs, and there is no recovering from that from inside the session it
    -- breaks. So the release is one file that runs anywhere.
    oslo.env.set("RUSTFLAGS", "-C target-feature=+crt-static")
    local features = a.type == "minimal" and {} or { "--all-features" }
    cargo("build", "--release", "--target", TARGET, "--bin", NAME, features)
    make.run("check-static")
    print(("%s  %.2f MB"):format(BIN, oslo.fs.stat(BIN).size / 1048576))
  end,
}

make.alias("b", "build")

make.recipe{
  name = "check-static",
  desc = "fail if the release ELF asks for a loader",
  run = function()
    -- "Static" is a claim about the ELF, so check the ELF. `ldd` is not enough: it prints
    -- "statically linked" for a musl binary that still has an INTERP and will not start.
    local segments = oslo.run{ "readelf", "-l", BIN, capture = true }
    assert(not segments.out:find("program interpreter"),
           BIN .. " requests a dynamic loader; it is not static")
    local dynamic = oslo.run{ "readelf", "-d", BIN, capture = true }
    assert(not (dynamic.out or ""):find("NEEDED"),
           BIN .. " has NEEDED entries; it is not static")
    print("static: no INTERP, no NEEDED")
  end,
}

-- Nothing to do, and that is the point: `build` depends on it so that a missing toolchain is one
-- clear message rather than a wall of linker output.
make.recipe{
  name = "_static-check-deps",
  desc = "the tools the static link needs",
  run = function()
    for _, tool in ipairs({ "cargo", "readelf" }) do
      assert(oslo.run{ "sh", "-c", "command -v " .. tool }.ok,
             tool .. " is not installed, and the static build needs it")
    end
  end,
}

make.recipe{ name = "dev", desc = "the fast inner loop: a debug build",
             run = function() cargo("build") end }

make.recipe{ name = "clean", desc = "remove the Cargo artifacts",
             run = function() cargo("clean") end }

make.recipe{ name = "compile", desc = "clean, then build", deps = { "clean", "build" } }
make.alias("c", "compile")

---------------------------------------------------------------------------- the gate

make.recipe{ name = "check", desc = "cargo check, every target",
             run = function() cargo("check", "--all-targets", OURS) end }

make.recipe{ name = "check-all", desc = "cargo check, every target and every feature",
             run = function() cargo("check", "--all-targets", "--all-features") end }

-- `--all-features`, like `check` and `clippy` beside it. Without it the tests behind a feature are
-- compiled and then never *run*, so a suite that looked green covered neither.
make.recipe{ name = "test", desc = "the suite: every target, every feature, our crates",
             run = function() cargo("test", "--all-targets", "--all-features", OURS) end }
make.alias("t", "test")

make.recipe{ name = "test-terminal", desc = "the PTY transcript tests alone",
             run = function() cargo("test", "--test", "terminal_semantics_tests") end }

make.recipe{ name = "clippy", desc = "clippy, warnings denied",
             run = function()
               cargo("clippy", "--all-targets", "--all-features", OURS, "--", "-D", "warnings")
             end }

make.recipe{ name = "fmt", desc = "format the workspace",
             run = function() cargo("fmt", "--all") end }

make.recipe{ name = "fmt-check", desc = "fail if anything is unformatted",
             run = function() cargo("fmt", "--all", "--", "--check") end }

make.recipe{ name = "rustdoc", desc = "build the docs, warnings denied",
             run = function()
               oslo.env.set("RUSTDOCFLAGS", "-Dwarnings")
               cargo("doc", "--all-features", "--no-deps", OURS)
             end }

make.recipe{ name = "check-loc", desc = "fail if any source file exceeds 600 lines",
             run = function() sh.sh("./scripts/check-loc.sh") end }

make.recipe{ name = "check-readme", desc = "fail if the readme names a file that does not exist",
             run = function() sh.sh("./scripts/check-readme.sh") end }

make.recipe{
  name = "verify",
  desc = "the whole local gate",
  deps = { "fmt-check", "check-loc", "check-readme", "check", "test", "clippy", "rustdoc" },
}

---------------------------------------------------------------------------- installing

make.recipe{
  name = "install",
  desc = "put the release binary in $PREFIX/bin",
  deps = { "build" },
  run = function()
    local dest = (os.getenv("DESTDIR") or "") .. PREFIX .. "/bin"
    sh.install("-d", dest)
    sh.install("-m", "755", BIN, dest .. "/" .. NAME)
    print("installed " .. dest .. "/" .. NAME)
  end,
}

make.recipe{
  name = "uninstall",
  desc = "take it back out of $PREFIX/bin",
  run = function()
    local dest = (os.getenv("DESTDIR") or "") .. PREFIX .. "/bin/" .. NAME
    sh.rm("-f", dest)
    print("removed " .. dest)
  end,
}

---------------------------------------------------------------------------- the VMs

-- Deliberately not in `verify`: each needs a musl toolchain, qemu and the network, and takes
-- minutes. They answer questions a checkout cannot — whether the artifact runs as PID 1 on a
-- foreign userland, and whether a distro's own init system runs on it.
--
-- The two distros disagree with each other on purpose. Alpine is musl, OpenRC and a busybox
-- `/bin/sh`; Arch is glibc, systemd and a *bash* `/bin/sh`, so standing in for it is a
-- bash-compatibility test rather than a POSIX one.
for _, vm in ipairs({
  { "vm", "scripts/alpine-vm.sh", "boot oslo as PID 1 in an Alpine minirootfs" },
  { "vm-distro", "scripts/alpine-distro-vm.sh", "a real Alpine userland with oslo as /bin/sh" },
  { "vm-arch", "scripts/arch-vm.sh", "glibc, systemd, and a bash /bin/sh" },
}) do
  make.recipe{ name = vm[1], desc = vm[3], run = function() sh.bash(vm[2]) end }
end

---------------------------------------------------------------------------- releasing

make.recipe{
  name = "release",
  desc = "cut a version: --type patch | minor | major | M.m.p",
  params = { { "--type", desc = "patch | minor | major | M.m.p" } },
  run = function(a)
    assert(oslo.run{ "sh", "-c", "command -v git-rel" }.ok,
           "git-rel is not installed; install it first")
    assert(type(a.type) == "string",
           "which release? oslo make release --type patch|minor|major|M.m.p")
    sh.git("rel", a.type)
  end,
}

---------------------------------------------------------------------------- the corpus

-- **Never run the corpus from the repository root.** These are real scripts and many of them
-- create files where they are run; doing it by hand has twice left ~70 stray files in the tree,
-- and one of them dropping a file called `f` changes what a *different* script does afterwards.
make.recipe{
  name = "corpus",
  desc = "time the script corpus, in a scratch directory",
  run = function()
    cargo("build", "--release")
    local root = oslo.fs.cwd()
    local dir = oslo.fs.mktempdir()
    for _, script in ipairs(oslo.fs.glob("tests/corpus/*.sh")) do
      oslo.fs.copy(script, dir .. "/" .. oslo.fs.name(script))
    end
    local scripts = oslo.fs.glob(dir .. "/*.sh")
    local began = os.time()
    for _, script in ipairs(scripts) do
      oslo.run{ root .. "/target/release/" .. NAME, script }
    end
    print(("corpus: %d scripts in %ds"):format(#scripts, os.time() - began))
    oslo.fs.remove(dir)
  end,
}
