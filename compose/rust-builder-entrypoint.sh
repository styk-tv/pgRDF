#!/usr/bin/env bash
# ck-build — SPEC.RUST-BUILDER.CK.v3.11 §3/§5/§7.
#
# Reads a pgrx crate from a read-only /src mount, resolves cargo-pgrx
# from that crate's OWN pin, packages it, and writes the artifact set
# atomically to /out.
#
#   /src     repo, read-only
#   /out     delivery target (the pgck-localhost ext volume)
#   /target  per-repo cargo target dir (writable; /src is ro)
#   /pgrx    version-scoped cargo-pgrx installs + PGRX_HOME (shared cache)
set -euo pipefail

# package (default) -> build and deliver to /out
# test              -> run the pgrx test suite, deliver nothing
#
# `test` exists so the suite runs through the SAME toolchain that produces the
# artifact. Running it by hand with `--entrypoint bash` silently omits the
# version-scoped cargo-pgrx from PATH; the pgrx harness then fails at its
# internal `cargo pgrx install` and reports it as ~300 failing tests, which
# reads like a broken change rather than a missing PATH.
MODE="${1:-package}"
PG_MAJOR="${PG_MAJOR:-18}"
SRC=/src
OUT=/out
say() { printf '  %-22s %s\n' "$1" "$2"; }
die() { printf '\n!! %s\n' "$*" >&2; exit 1; }

echo "ck-build · SPEC.RUST-BUILDER.CK.v3.11"
[ -f "$SRC/Cargo.toml" ] || die "no Cargo.toml at $SRC — mount the repo read-only at /src"

# ---- identity -------------------------------------------------------------
EXT=$(sed -n '/^\[package\]/,/^\[/p' "$SRC/Cargo.toml" | sed -n 's/^name *= *"\(.*\)"/\1/p' | head -1)
VER=$(sed -n '/^\[package\]/,/^\[/p' "$SRC/Cargo.toml" | sed -n 's/^version *= *"\(.*\)"/\1/p' | head -1)
[ -n "$EXT" ] || die "could not read package name from $SRC/Cargo.toml"

# Git identity is what makes the artifact attributable. A dirty tree
# means the .so corresponds to no commit anyone else can fetch.
#
# /src is owned by the host user and we run as root, so git refuses it
# as "dubious ownership" unless told otherwise. That refusal must not
# be allowed to LOOK like a clean tree: `git status | wc -l` returns 0
# both when the tree is clean and when git never ran, and recording
# the second as `"dirty": false` is a false provenance claim. So we
# clear the ownership check first, then treat an unusable git as
# UNKNOWN — never as clean.
git config --global --add safe.directory "$SRC" 2>/dev/null || true

if git -C "$SRC" rev-parse --git-dir >/dev/null 2>&1; then
  COMMIT=$(git -C "$SRC" rev-parse --short HEAD)

  # Dirtiness means TRACKED files differ from HEAD. Two container-specific
  # traps make the naive check wrong in opposite ways:
  #
  # 1. `git status --porcelain` counts UNTRACKED files. The host's ignore
  #    rules (global excludesFile, .git/info/exclude) do not come along
  #    into the container, so files ignored on the host show up as `??`
  #    here and every build reports dirty. That also makes
  #    CK_REQUIRE_CLEAN refuse every release build.
  #
  # 2. /src is READ-ONLY, so git cannot write back a refreshed index.
  #    Stat metadata differs across the mount, so `diff-index` reports
  #    modifications for files whose contents are identical.
  #
  # Both are fixed by refreshing into a WRITABLE copy of the index and
  # comparing tracked content only.
  GIT_INDEX_COPY=/tmp/ck-build-index
  if cp "$(git -C "$SRC" rev-parse --git-dir)/index" "$GIT_INDEX_COPY" 2>/dev/null; then
    if GIT_INDEX_FILE="$GIT_INDEX_COPY" git -C "$SRC" update-index --refresh -q >/dev/null 2>&1 \
       || true; then
      GIT_INDEX_FILE="$GIT_INDEX_COPY" git -C "$SRC" diff-index --quiet HEAD -- \
        && DIRTYB=false || DIRTYB=true
    fi
    rm -f "$GIT_INDEX_COPY"
  else
    # No readable index — do not guess. UNKNOWN, never clean.
    DIRTYB=null
  fi
else
  COMMIT=unknown
  DIRTYB=null          # JSON null — "not known", distinct from false
  echo "  NOTE  $SRC is not a usable git repo — provenance records dirty: null"
fi

# The release path sets CK_REQUIRE_CLEAN=1; bench builds default to
# recording the truth and carrying on, because building uncommitted
# work is the normal development case.
if [ "${CK_REQUIRE_CLEAN:-0}" = "1" ] && [ "$DIRTYB" != "false" ]; then
  die "CK_REQUIRE_CLEAN=1 and the tree is dirty or unattributable (dirty: $DIRTYB)"
fi

# ---- toolchain resolution (§3) --------------------------------------------
# EXACT pin from Cargo.lock, never a caret: a range resolves against
# whatever is published at build time and drifts from the crate silently.
PIN=$(awk '/^name = "pgrx"$/{f=1;next} f&&/^version/{gsub(/[",]/,"");print $3;exit}' "$SRC/Cargo.lock" 2>/dev/null || true)
[ -n "$PIN" ] || die "could not resolve the pgrx pin from $SRC/Cargo.lock"

# Cargo.lock must already agree with Cargo.toml. /src is read-only, so a
# stale lock makes cargo fail deep inside cargo-pgrx with
# "Read-only file system (os error 30)" and a metadata.rs backtrace, which
# names neither the lock file nor the version. Every version bump hits this.
# Refuse here, with the actual cause.
LOCKVER=$(awk -v n="$EXT" '$0=="name = \""n"\""{f=1;next} f&&/^version/{gsub(/[",]/,"");print $3;exit}' "$SRC/Cargo.lock" 2>/dev/null || true)
if [ -n "$LOCKVER" ] && [ "$LOCKVER" != "$VER" ]; then
  die "Cargo.lock says ${EXT} ${LOCKVER}, Cargo.toml says ${VER}. /src is read-only so cargo cannot reconcile them — update and commit Cargo.lock first."
fi

RUSTC=$(rustc --version | awk '{print $2}')
say "extension" "$EXT $VER"
say "commit" "$COMMIT (dirty: $DIRTYB)"
say "pgrx pin" "$PIN"
say "rustc" "$RUSTC"
say "pg_major" "$PG_MAJOR"

PGRX_ROOT="/pgrx/cargo-pgrx-${PIN}"
if [ ! -x "${PGRX_ROOT}/bin/cargo-pgrx" ]; then
  echo "  installing cargo-pgrx =${PIN} (version-scoped, cached)"
  cargo install cargo-pgrx --locked --version "=${PIN}" --root "${PGRX_ROOT}"
else
  say "cargo-pgrx" "cached at ${PGRX_ROOT}"
fi
export PATH="${PGRX_ROOT}/bin:$PATH"

mkdir -p "${PGRX_HOME:-/pgrx/home}"
if [ ! -f "${PGRX_HOME:-/pgrx/home}/config.toml" ]; then
  cargo pgrx init "--pg${PG_MAJOR}" "$(command -v pg_config)"
fi

# ---- build ----------------------------------------------------------------
# Build identity, injected at compile time as <EXT>_BUILD_ID (PGRDF_BUILD_ID,
# PGCK_BUILD_ID, ...). A crate that reads it can answer "which build is this"
# from inside SQL; one that ignores it is unaffected. Deliberately carries
# only tag/commits-since/short-commit/dirty — no paths, host or user, because
# any connected role can read it.
BUILD_ID_VAR="$(printf '%s' "$EXT" | tr '[:lower:]-' '[:upper:]_')_BUILD_ID"
# Caller-supplied identity wins (#112): CI passes the tag explicitly because
# its checkout is shallow and tagless — git describe there would lie or starve.
PRESET_BUILD_ID=$(eval "printf '%s' \"\${${BUILD_ID_VAR}:-}\"")
if [ -n "$PRESET_BUILD_ID" ]; then
  BUILD_ID="$PRESET_BUILD_ID"
elif [ "$COMMIT" != "unknown" ]; then
  BUILD_ID=$(git -C "$SRC" describe --tags --always --dirty 2>/dev/null || echo "$COMMIT")
else
  BUILD_ID=unknown
fi
export "$BUILD_ID_VAR=$BUILD_ID"
say "build id" "$BUILD_ID  (as \$$BUILD_ID_VAR)"

cd "$SRC"

if [ "$MODE" = "test" ]; then
  say "mode" "test — running the pgrx suite, delivering nothing"
  # `initdb` refuses to run as root, and this container is root. CI never hits
  # it because a GitHub runner is already unprivileged; here the suite would
  # fail 296 tests on one initdb refusal, every downstream test reporting
  # "could not obtain test mutex" rather than the actual cause.
  #
  # pgrx supports an unprivileged run via CARGO_PGRX_TEST_RUNAS; the image
  # provisions `postgres` with NOPASSWD sudo for exactly this. PGDATA moves off
  # the root-owned /target volume so the sudo'd mkdir can write.
  export CARGO_PGRX_TEST_RUNAS=postgres
  export CARGO_PGRX_TEST_PGDATA=/tmp/pgrx-pgdata
  mkdir -p "$CARGO_PGRX_TEST_PGDATA"
  chown -R postgres:postgres "$CARGO_PGRX_TEST_PGDATA"
  chmod -R a+rX "${PGRX_HOME:-/pgrx/home}" 2>/dev/null || true
  say "runas" "postgres (initdb cannot run as root)"
  cargo pgrx test --no-default-features --features "pg${PG_MAJOR}" "pg${PG_MAJOR}"
  exit $?
fi

cargo pgrx package --pg-config "$(command -v pg_config)"

PKG="${CARGO_TARGET_DIR:-/target}/release/${EXT}-pg${PG_MAJOR}"
SO="${PKG}/usr/lib/postgresql/${PG_MAJOR}/lib/${EXT}.so"
SHARE="${PKG}/usr/share/postgresql/${PG_MAJOR}/extension"
[ -f "$SO" ] || die "no .so produced at $SO"

# ---- delivery (§5) --------------------------------------------------------
# .so and SQL move together or not at all: a binary whose catalog entry
# is a different generation gives mismatched function arities.
STAGE=$(mktemp -d "${OUT}/.ck-build.XXXXXX")
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/lib" "$STAGE/share/extension"
cp "$SO" "$STAGE/lib/${EXT}.so"
cp "$SHARE/${EXT}.control" "$STAGE/share/extension/"
cp "$SHARE"/*.sql "$STAGE/share/extension/"

# Upgrade scripts, explicitly from the crate's sql/ (#94).
#
# `cargo pgrx package` copies these on some versions and not others -- pgRDF's
# release workflow carries a comment stating it does NOT, which was true when
# written and is not true on 0.19.2, where the package step is observed copying
# pgrdf--0.5.1--0.6.20.sql. A delivery contract that silently depends on which
# pgrx a crate pins is not a contract, so copy them here regardless. Duplicate
# copies are identical files and harmless.
#
# Without them a delivered extension can only ever be installed FRESH: postgres
# refuses to apply a full install to an already-installed extension, so a
# database carrying data has no route forward and ALTER EXTENSION fails with
# "no update path".
if compgen -G "$SRC/sql/${EXT}--*--*.sql" > /dev/null 2>&1; then
  cp "$SRC"/sql/"${EXT}"--*--*.sql "$STAGE/share/extension/"
  say "upgrade sql" "$(ls -1 "$SRC"/sql/"${EXT}"--*--*.sql | wc -l | tr -d ' ') script(s) from sql/"
else
  say "upgrade sql" "none in sql/ — a surface change will need one"
fi

( cd "$STAGE/lib" && sha256sum "${EXT}.so" > "${EXT}.so.sha256" )
DIGEST=$(awk '{print $1}' "$STAGE/lib/${EXT}.so.sha256")

cat > "$STAGE/${EXT}.build.json" <<EOF
{
  "extension": "${EXT}",
  "version": "${VER}",
  "commit": "${COMMIT}",
  "dirty": ${DIRTYB},
  "pgrx": "${PIN}",
  "rustc": "${RUSTC}",
  "build_id": "${BUILD_ID}",
  "base": "debian-13-trixie",
  "pg_major": ${PG_MAJOR},
  "so_sha256": "${DIGEST}",
  "built_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
EOF

mkdir -p "$OUT/lib" "$OUT/share/extension"
mv -f "$STAGE/lib/${EXT}.so" "$STAGE/lib/${EXT}.so.sha256" "$OUT/lib/"
mv -f "$STAGE/share/extension/"* "$OUT/share/extension/"
mv -f "$STAGE/${EXT}.build.json" "$OUT/"

echo
say "so" "$(stat -c%s "$OUT/lib/${EXT}.so") bytes"
say "sha256" "${DIGEST:0:16}"
say "delivered" "$OUT/lib/${EXT}.so + share/extension + build.json"
[ "$DIRTYB" = "false" ] || echo "  NOTE  dirty: $DIRTYB — this artifact is not attributable to a commit"
exit 0
