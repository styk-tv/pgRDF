SHELL := bash

EXTENSION   := pgrdf
EXTVERSION  := $(shell sed -n "s/^default_version = '\(.*\)'/\1/p" pgrdf.control)
METAVERSION := $(shell sed -n 's/^[[:space:]]*"version":[[:space:]]*"\([^"]*\)",\{0,1\}[[:space:]]*$$/\1/p' META.json | head -n 1)
DISTVERSION := $(EXTVERSION)
DISTFILE    := $(EXTENSION)-$(DISTVERSION).zip
DIST_PATHS  := \
	META.json \
	Makefile \
	LICENSE \
	README.pgxn.md \
	INSTALL.md \
	pgrdf.control \
	Cargo.toml \
	Cargo.lock \
	rust-toolchain.toml \
	src \
	sql \
	examples

PG_CONFIG   ?= pg_config
PG_MAJOR    := $(shell "$(PG_CONFIG)" --version | sed -E 's/.* ([0-9]+)(\..*)?/\1/')
PKGLIBDIR   := $(shell "$(PG_CONFIG)" --pkglibdir)
SHAREDIR    := $(shell "$(PG_CONFIG)" --sharedir)

PACKAGE_DIR    := target/release/$(EXTENSION)-pg$(PG_MAJOR)
PACKAGE_LIB    := $(PACKAGE_DIR)/usr/lib/postgresql/$(PG_MAJOR)/lib/$(EXTENSION).so
PACKAGE_EXTDIR := $(PACKAGE_DIR)/usr/share/postgresql/$(PG_MAJOR)/extension

.PHONY: all check-meta check-tools package install installcheck dist clean

all: package

check-meta:
	@test -n "$(EXTVERSION)" || { echo "could not read default_version from pgrdf.control" >&2; exit 1; }
	@test -n "$(METAVERSION)" || { echo "could not read version from META.json" >&2; exit 1; }
	@test "$(EXTVERSION)" = "$(METAVERSION)" || { echo "version mismatch: pgrdf.control=$(EXTVERSION) META.json=$(METAVERSION)" >&2; exit 1; }

check-tools:
	@command -v cargo >/dev/null 2>&1 || { echo "cargo is required" >&2; exit 1; }
	@cargo pgrx --version >/dev/null 2>&1 || { echo "cargo-pgrx is required; install cargo-pgrx 0.16 and run cargo pgrx init first" >&2; exit 1; }
	@command -v "$(PG_CONFIG)" >/dev/null 2>&1 || { echo "pg_config not found: $(PG_CONFIG)" >&2; exit 1; }

package: check-meta check-tools
	cargo pgrx package --pg-config "$(PG_CONFIG)"

install: package
	install -d "$(DESTDIR)$(PKGLIBDIR)" "$(DESTDIR)$(SHAREDIR)/extension"
	install -m 755 "$(PACKAGE_LIB)" "$(DESTDIR)$(PKGLIBDIR)/$(EXTENSION).so"
	install -m 644 "$(PACKAGE_EXTDIR)"/*.control "$(DESTDIR)$(SHAREDIR)/extension/"
	install -m 644 "$(PACKAGE_EXTDIR)"/*.sql "$(DESTDIR)$(SHAREDIR)/extension/"

installcheck: check-meta check-tools
	cargo pgrx test --no-default-features --features pg$(PG_MAJOR) pg$(PG_MAJOR)

dist: check-meta
	@tmpdir="$$(mktemp -d .pgxn-dist.XXXXXX)"; \
	repo_root="$$(pwd)"; \
	trap 'rm -rf "$$tmpdir"' EXIT; \
	mkdir -p "$$tmpdir/$(EXTENSION)-$(DISTVERSION)"; \
	rm -f "$$repo_root/$(DISTFILE)"; \
	for path in $(DIST_PATHS); do \
		[ -e "$$path" ] || { echo "missing dist path: $$path" >&2; exit 1; }; \
		mkdir -p "$$tmpdir/$(EXTENSION)-$(DISTVERSION)/$$(dirname "$$path")"; \
		cp -R "$$path" "$$tmpdir/$(EXTENSION)-$(DISTVERSION)/$$path"; \
	done; \
	( cd "$$tmpdir" && zip -qr "$$repo_root/$(DISTFILE)" "$(EXTENSION)-$(DISTVERSION)" )

clean:
	cargo clean
	rm -f "$(DISTFILE)"
