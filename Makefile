SHELL := /bin/bash

# Delegated to a script on purpose — see the comment at the top of it. Parsing PROJECT inline here
# needs a literal `#`, which older GNU Make mis-parses inside $(shell ...).
PROJECT_META := $(shell $(CURDIR)/scripts/project-meta.sh)
PROJECT_NAME := $(word 1,$(PROJECT_META))
PROJECT_VERSION := $(word 2,$(PROJECT_META))
ifeq ($(PROJECT_NAME),)
    $(error Error: PROJECT file not found or invalid)
endif

TOP_DIR := $(CURDIR)
CARGO := cargo

# What a release is: one file that runs anywhere, with no loader and no libc to find.
#
# `make build` produces exactly what the release workflow produces, and for a reason beyond
# tidiness — oslo is meant to be somebody's *login shell*, and a login shell linked against a
# /nix/store glibc stops existing the day `nix-collect-garbage` runs. There is no recovering from
# that from inside the session it breaks.
#
# `RUSTFLAGS` and the deliberately-absent linker override are copied from
# `.github/workflows/release.yml`; the comment there explains why pointing this at `musl-gcc`
# silently produces a *dynamic* musl binary.
TARGET ?= x86_64-unknown-linux-musl
STATIC_RUSTFLAGS := -C target-feature=+crt-static
BIN := target/$(TARGET)/release/$(PROJECT_NAME)
PREFIX ?= $(HOME)/.local

HAS_REL := $(shell command -v git-rel 2>/dev/null)

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info ------------------------------------------)

.PHONY: build build-all b dev check-static compile c run r test test-terminal t check check-all test-all check-loc check-readme print-name clippy rustdoc fmt fmt-check clean verify vm vm-distro vm-arch install uninstall release help h

build:
	@RUSTFLAGS="$(STATIC_RUSTFLAGS)" $(CARGO) build --release --target $(TARGET) --bin $(PROJECT_NAME)
	@$(MAKE) --no-print-directory check-static
	@ls -l "$(BIN)" | awk '{printf "%s  %.2f MB\n", $$NF, $$5/1048576}'

b: build

# The same binary with every optional feature switched on.
#
# `--all-features` means what it says here and nothing more, because nothing in this workspace has
# a feature that exists to serve tests: the two that did are ordinary `pub` items now, reachable by
# `cargo test` across crates and dropped from the binary by the linker because nothing else calls
# them. A build flag should never decide whether test scaffolding ships.
build-all:
	@RUSTFLAGS="$(STATIC_RUSTFLAGS)" $(CARGO) build --release --target $(TARGET) --bin $(PROJECT_NAME) --all-features
	@$(MAKE) --no-print-directory check-static
	@ls -l "$(BIN)" | awk '{printf "%s  %.2f MB\n", $$NF, $$5/1048576}'

# "Static" is a claim about the ELF, so check the ELF. `ldd` is not enough: it prints
# "statically linked" for a musl binary that still has an INTERP and will not start.
check-static:
	@bin="$(BIN)"; \
	if readelf -l "$$bin" | grep -q 'program interpreter'; then \
		echo "error: $$bin requests a dynamic loader; it is not static" >&2; \
		readelf -l "$$bin" | grep 'program interpreter' >&2; \
		exit 1; \
	fi; \
	if readelf -d "$$bin" 2>/dev/null | grep -q NEEDED; then \
		echo "error: $$bin has NEEDED entries; it is not static" >&2; \
		readelf -d "$$bin" | grep NEEDED >&2; \
		exit 1; \
	fi; \
	echo "static: no INTERP, no NEEDED"

# The fast inner loop. `build` is what ships; this is what you run while writing code.
dev:
	@$(CARGO) build

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run:
	@$(CARGO) run --bin $(PROJECT_NAME) -- $(ARGS)

r: run

# oslo's own crates, and not the vendored ones.
#
# **Not plain `--workspace`.** The crates under `vendor/` carry their upstream test modules, whose
# dev-dependencies were never vendored, so `--workspace` fails to compile before it runs anything
# of ours. Nor is it plain `cargo test`: that is the root package alone, and the moment code moved
# into `crates/` its tests silently stopped running — a suite that quietly shrinks is worse than
# one that fails. Excluding by name states which code is somebody else's; see `vendor/README.md`.
OURS := --workspace --exclude brush-parser --exclude full_moon --exclude full_moon_derive

test:
	@$(CARGO) test --all-targets $(OURS)

test-terminal:
	@$(CARGO) test --test terminal_semantics_tests

t: test

check:
	@$(CARGO) check --all-targets $(OURS)

check-all:
	@$(CARGO) check --all-targets --all-features

fmt:
	@$(CARGO) fmt --all

fmt-check:
	@$(CARGO) fmt --all -- --check

clippy:
	@$(CARGO) clippy --all-targets --all-features $(OURS) -- -D warnings

rustdoc:
	@RUSTDOCFLAGS="-Dwarnings" $(CARGO) doc --all-features --no-deps $(OURS)

test-all:
	@$(CARGO) test --all-targets --all-features

clean:
	@$(CARGO) clean

# Echoes the name the Makefile parsed out of PROJECT. CI prints it because an empty name trips the
# $(error) at the top of this file before any target runs, and that failure looks like nothing at
# all in a run summary — every step just fails instantly. Printing the value names the cause.
print-name:
	@echo '$(PROJECT_NAME)'

check-loc:
	@./scripts/check-loc.sh

check-readme:
	@./scripts/check-readme.sh

# Both optional features — `ssh` and `vista` — are *compiled* by the gate, because `clippy` and
# `rustdoc` run `--all-features`. `check-all` is kept for running that alone.
#
# `verify` still runs plain `check` and `test`, which is deliberate: the shipped artifact is the
# default build, and a gate that only ever exercised `--all-features` would stop testing the thing
# people actually get.
#
# **`vista` has tests that plain `verify` does not run**, unlike `ssh`, which has nothing to test
# yet. They are one command, and worth it before touching the model or the editor's hint path:
#
#     PATH=/tmp/realbash:$PATH cargo test --features vista --all-targets $(OURS)
#
# The VMs are deliberately *not* in `verify`: each needs a musl toolchain, qemu and the network,
# and takes minutes. They answer questions a checkout cannot — whether the release artifact runs as
# PID 1 on a foreign userland, and whether a distro's own init system runs on it.
#
# The two distros are chosen to disagree with each other. Alpine is musl, OpenRC and a busybox
# `/bin/sh`; Arch is glibc, systemd and a **bash** `/bin/sh`, so standing in for it is a
# bash-compatibility test rather than a POSIX one. Passing both is worth far more than passing
# either twice.
vm:
	bash scripts/alpine-vm.sh

vm-distro:
	bash scripts/alpine-distro-vm.sh

vm-arch:
	bash scripts/arch-vm.sh

# Time the corpus, in a scratch directory.
#
# **Never run the corpus from the repository root.** These are real scripts and many of them create
# files where they are run; doing it by hand has twice now left ~70 stray files in the tree and in
# `tests/corpus/` itself, and one of them dropping a file called `f` changes what a *different*
# script does afterwards. `tests/differential_tests.rs` and
# `tests/posix_stays_on_the_byte_path.rs` already sandbox for exactly that reason — this target is
# the same courtesy for a human with a stopwatch.
corpus:
	@cargo build --release
	@dir=$$(mktemp -d) && cp tests/corpus/*.sh "$$dir"/ && cd "$$dir" && \
		start=$$(date +%s.%N); \
		for f in *.sh; do "$(CURDIR)/target/release/oslo" "$$f" >/dev/null 2>&1; done; \
		end=$$(date +%s.%N); \
		printf 'corpus: %d scripts in %.2fs\n' "$$(ls *.sh | wc -l)" "$$(echo "$$end - $$start" | bc)"; \
		cd / && rm -rf "$$dir"

verify: fmt-check check-loc check-readme check test clippy rustdoc

install: build
	@install -d "$(DESTDIR)$(PREFIX)/bin"
	@install -m 755 "$(BIN)" "$(DESTDIR)$(PREFIX)/bin/$(PROJECT_NAME)"
	@echo "installed $(DESTDIR)$(PREFIX)/bin/$(PROJECT_NAME)"

uninstall:
	@rm -f "$(DESTDIR)$(PREFIX)/bin/$(PROJECT_NAME)"
	@echo "removed $(DESTDIR)$(PREFIX)/bin/$(PROJECT_NAME)"

release:
	@if [ -z "$(HAS_REL)" ]; then \
		echo "git-rel is not installed. Please install it first."; \
		exit 1; \
	fi
	@if [ -z "$(TYPE)" ]; then \
		echo "Release type not specified. Use 'make release TYPE=[patch|minor|major|M.m.p]'"; \
		exit 1; \
	fi
	@git rel $(TYPE)

help:
	@echo
	@echo "Usage: make [target]"
	@echo
	@echo "Available targets:"
	@echo "  build        Build the release binary"
	@echo "  compile      Clean and rebuild"
	@echo "  run          Run the shell (make run ARGS='-c \"echo hi\"')"
	@echo "  test         Run all tests"
	@echo "  test-terminal Run terminal PTY transcript tests"
	@echo "  check        Run cargo check on all targets"
	@echo "  build-all    Static release with every optional feature on"
	@echo "  check-all    Run cargo check on all targets/all features"
	@echo "  test-all     Run cargo test on all targets/all features"
	@echo "  clippy       Run clippy with warnings denied"
	@echo "  rustdoc      Build docs with warnings denied"
	@echo "  fmt          Format the workspace"
	@echo "  fmt-check    Check formatting"
	@echo "  check-loc    Fail if any source file exceeds 600 lines"
	@echo "  check-readme Fail if the README names a file that does not exist"
	@echo "  clean        Remove Cargo build artifacts"
	@echo "  verify       Run the full local gate"
	@echo "  vm           Boot oslo as PID 1 in an Alpine minirootfs and run its suites"
	@echo "  vm-distro    Boot a real Alpine userland with oslo as /bin/sh and run OpenRC"
	@echo "  install      Install the release binary into \$$PREFIX/bin ($(PREFIX)/bin)"
	@echo "  uninstall    Remove the installed binary from \$$PREFIX/bin"
	@echo "  release      Release a new version"
	@echo

h: help
