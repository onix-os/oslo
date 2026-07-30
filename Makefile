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
EXAMPLE ?= main
PREFIX ?= $(HOME)/.local

HAS_REL := $(shell command -v git-rel 2>/dev/null)

$(info ------------------------------------------)
$(info Project: $(PROJECT_NAME) v$(PROJECT_VERSION))
$(info ------------------------------------------)

.PHONY: build b compile c run r example test t check check-all test-all check-loc check-readme print-name clippy rustdoc fmt fmt-check clean verify vm vm-distro install uninstall release help h

build:
	@$(CARGO) build

b: build

compile:
	@$(CARGO) clean
	@$(MAKE) build

c: compile

run:
	@$(CARGO) run --bin $(PROJECT_NAME) -- $(ARGS)

r: run

example:
	@$(CARGO) run --example $(EXAMPLE)

test:
	@$(CARGO) test --all-targets

t: test

check:
	@$(CARGO) check --all-targets

check-all:
	@$(CARGO) check --all-targets --all-features

fmt:
	@$(CARGO) fmt --all

fmt-check:
	@$(CARGO) fmt --all -- --check

clippy:
	@$(CARGO) clippy --all-targets --all-features -- -D warnings

rustdoc:
	@RUSTDOCFLAGS="-Dwarnings" $(CARGO) doc --all-features --no-deps

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

# check-all/test-all are deliberately absent: the crate declares no [features], so they are
# byte-identical reruns of check/test and only slow the gate down. Add them back the day a
# [features] section appears.
# The two VMs are deliberately *not* in `verify`: each needs a musl toolchain, qemu and the
# network, and takes minutes. They answer questions a checkout cannot — whether the release
# artifact runs as PID 1 on a foreign userland, and whether a distro's own init system runs on it.
vm:
	bash scripts/alpine-vm.sh

vm-distro:
	bash scripts/alpine-distro-vm.sh

verify: fmt-check check-loc check-readme check test clippy rustdoc

install:
	@$(CARGO) build --release --bin $(PROJECT_NAME)
	@install -d "$(DESTDIR)$(PREFIX)/bin"
	@install -m 755 "target/release/$(PROJECT_NAME)" "$(DESTDIR)$(PREFIX)/bin/$(PROJECT_NAME)"
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
	@echo "  build        Build the shell and library"
	@echo "  compile      Clean and rebuild"
	@echo "  run          Run the shell (make run ARGS='-c \"echo hi\"')"
	@echo "  example      Run a development example (make example EXAMPLE=main)"
	@echo "  test         Run all tests"
	@echo "  check        Run cargo check on all targets"
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

