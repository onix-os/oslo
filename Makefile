SHELL := /bin/bash

PROJECT_NAME := $(shell if [ -f PROJECT ]; then sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | head -1 | tr -d '[:space:]'; else sed -n 's/^[[:space:]]*name[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -1; fi)
PROJECT_VERSION := $(shell if [ -f PROJECT ]; then sed -n '/^[[:space:]]*[^#\[[:space:]]/p' PROJECT | sed -n '2p' | tr -d '[:space:]'; else sed -n 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' Cargo.toml | head -1; fi)
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

.PHONY: build b compile c run r example test t check check-all test-all check-loc clippy rustdoc fmt fmt-check clean verify install uninstall release help h

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

check-loc:
	@./scripts/check-loc.sh

# check-all/test-all are deliberately absent: the crate declares no [features], so they are
# byte-identical reruns of check/test and only slow the gate down. Add them back the day a
# [features] section appears.
verify: fmt-check check-loc check test clippy rustdoc

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
	@echo "  clean        Remove Cargo build artifacts"
	@echo "  verify       Run the full local gate"
	@echo "  install      Install the release binary into \$$PREFIX/bin ($(PREFIX)/bin)"
	@echo "  uninstall    Remove the installed binary from \$$PREFIX/bin"
	@echo "  release      Release a new version"
	@echo

h: help

