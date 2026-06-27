#!/usr/bin/env bash
#
# validate-bundle.sh — validate a pgRDF release bundle's MANIFEST.json (#37).
#
# Checks the *static* version coherence of an extracted bundle (or .tar.gz):
# the manifest's version/extversion/runtime.value agree with each other and
# with the control file + the SQL filenames actually present. This is the
# "one place to read a component's version" a consumer (oci-germination's
# versions.yaml, a downstream bundle) validates against, instead of grepping
# filenames or pinning by hand.
#
# It does NOT boot Postgres — the runtime `pgrdf.version()` check is the
# release pipeline's job (oci-publish gate boots the .so and cross-checks the
# manifest's runtime.value against the actual booted version). This script is
# the cheap static gate a consumer can run on a pulled artifact.
#
# Usage:
#   tools/validate-bundle.sh <dir|tarball>           # validate
#   tools/validate-bundle.sh <dir|tarball> <ver>     # also assert version == <ver>
set -euo pipefail

SRC="${1:?usage: validate-bundle.sh <dir|tarball> [expected-version]}"
WANT="${2:-}"
command -v jq >/dev/null || { echo "validate-bundle: jq required"; exit 2; }

# Resolve to a layout directory (extract a tarball to a temp dir if needed).
if [ -d "${SRC}" ]; then
  DIR="${SRC}"
else
  TMP="$(mktemp -d)"; trap 'rm -rf "${TMP}"' EXIT
  tar xzf "${SRC}" -C "${TMP}"
  DIR="$(find "${TMP}" -maxdepth 1 -type d -name 'pgrdf-*' | head -1)"
  [ -n "${DIR}" ] || { echo "::error::no pgrdf-* layout dir inside ${SRC}"; exit 1; }
fi

MAN="${DIR}/MANIFEST.json"
[ -f "${MAN}" ] || { echo "::error::MANIFEST.json missing in ${DIR}"; exit 1; }
jq -e . "${MAN}" >/dev/null || { echo "::error::MANIFEST.json is not valid JSON"; exit 1; }

VER=$(jq -r '.version' "${MAN}")
EXTVER=$(jq -r '.extversion' "${MAN}")
RTVER=$(jq -r '.runtime.value' "${MAN}")
INSTALL=$(jq -r '.install_sql' "${MAN}")
CTL_DEFAULT=$(grep -E '^default_version' "${DIR}/share/extension/pgrdf.control" | head -1 | cut -d"'" -f2)

fail=0
[ "${EXTVER}" = "${VER}" ]      || { echo "::error::extversion(${EXTVER}) != version(${VER})"; fail=1; }
[ "${RTVER}" = "${VER}" ]       || { echo "::error::runtime.value(${RTVER}) != version(${VER})"; fail=1; }
[ "${CTL_DEFAULT}" = "${VER}" ] || { echo "::error::control default_version(${CTL_DEFAULT}) != version(${VER})"; fail=1; }
[ -f "${DIR}/share/extension/${INSTALL}" ] || { echo "::error::install_sql(${INSTALL}) absent from layout"; fail=1; }
[ -f "${DIR}/lib/pgrdf.so" ]    || { echo "::error::lib/pgrdf.so absent"; fail=1; }
if [ -n "${WANT}" ] && [ "${VER}" != "${WANT}" ]; then echo "::error::manifest version ${VER} != expected ${WANT}"; fail=1; fi

# Optional integrity: verify SHA256SUMS if sha256sum is available.
if command -v sha256sum >/dev/null && [ -f "${DIR}/SHA256SUMS" ]; then
  ( cd "${DIR}" && sha256sum -c --quiet SHA256SUMS ) || { echo "::error::SHA256SUMS mismatch"; fail=1; }
fi

[ "${fail}" -eq 0 ] || exit 1
echo "✓ bundle OK: pgrdf ${VER} (extversion=${EXTVER}, runtime=${RTVER}, $(jq -r '.platform.arch' "${MAN}")/pg$(jq -r '.platform.pg_major' "${MAN}"))"
