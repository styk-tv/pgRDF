#!/usr/bin/env bash
#
# tests/perf/lubm/generator/generate.sh — entrypoint inside the
# pgrdf-lubm-generator container.
#
# Generates a LUBM dataset of N universities (default 10) using the
# UBA generator, then converts the OWL/N3 output to N-Triples and
# Turtle for ingest via `pgrdf.parse_turtle` / `pgrdf.parse_nquads`.
#
# Output layout inside the docker volume /data:
#   /data/lubm-<N>/
#     ├── raw/             (UBA output, OWL/RDF-XML)
#     │   ├── University0_0.owl
#     │   ├── University0_1.owl
#     │   └── ...
#     ├── nt/              (converted N-Triples — one file per
#     │   ├── lubm-<N>.nt   university OR a single concatenated file)
#     └── ttl/             (converted Turtle)
#         └── lubm-<N>.ttl
#
# Usage (inside the container — `docker run -v pgrdf-lubm-data:/data`):
#   /usr/local/bin/generate [univ_count]
#
# Usage (from the host — bridged through `just lubm-gen N`):
#   docker run --rm -v pgrdf-lubm-data:/data pgrdf-lubm-generator:latest 10
set -euo pipefail

UNIV_COUNT="${1:-10}"
OUT_DIR="/data/lubm-${UNIV_COUNT}"
RAW_DIR="${OUT_DIR}/raw"
NT_DIR="${OUT_DIR}/nt"
TTL_DIR="${OUT_DIR}/ttl"

mkdir -p "${RAW_DIR}" "${NT_DIR}" "${TTL_DIR}"

echo "[pgrdf-lubm-generator] generating LUBM-${UNIV_COUNT} into ${OUT_DIR}"

# UBA 1.7's zip extracts straight to /opt/lubm/ (no top-level subdir).
# UBA emits filenames as `<cwd>\University<i>_<j>.owl` — the path
# separator is a *literal* backslash hardcoded into UBA's Java source
# (it does not honour os.file.separator on Linux). So we cd into the
# raw output dir and let the backslashed names land there; a post-rename
# strips the `_uba-work\` prefix so files are accessible as plain
# `University<i>_<j>.owl` for rapper.
WORK_DIR="${OUT_DIR}/_uba-work"
mkdir -p "${WORK_DIR}"
cd "${WORK_DIR}"

# The UBA generator wants `classes/` on the classpath and the
# Univ-Bench ontology as a file:// URI for the -onto argument. The
# ontology was fetched into /opt/lubm/univ-bench.owl by the Dockerfile.
# UBA 1.7 has no `-timestamp` flag (per /opt/lubm/readme.txt); -seed 0
# gives deterministic output.
java -cp /opt/lubm/classes \
  edu.lehigh.swat.bench.uba.Generator \
  -univ "${UNIV_COUNT}" \
  -onto "file:///opt/lubm/univ-bench.owl" \
  -seed 0

# UBA actually writes into the *parent* of cwd with a `<cwdname>\`
# filename prefix — so the files land at `${OUT_DIR}/_uba-work\<file>`,
# i.e. siblings of the work dir at OUT_DIR root. Move + rename strip
# that prefix and land them in raw/.
cd "${OUT_DIR}"
shopt -s nullglob
moved=0
for f in '_uba-work\University'*.owl; do
  bare="${f#_uba-work\\}"
  mv -- "${f}" "${RAW_DIR}/${bare}"
  moved=$((moved + 1))
done
if [ "${moved}" -eq 0 ]; then
  echo "[pgrdf-lubm-generator] FATAL: UBA produced no output files; aborting" >&2
  exit 1
fi
# UBA also drops a `_uba-work\log.txt` next to the .owl files; tidy.
rm -f -- '_uba-work\log.txt' 2>/dev/null || true
# Clean up the work scratch (UBA never wrote inside it).
rmdir "${WORK_DIR}" 2>/dev/null || true

# Convert OWL → N-Triples via raptor (rapper).
echo "[pgrdf-lubm-generator] converting to N-Triples"
shopt -s nullglob
NT_OUT="${NT_DIR}/lubm-${UNIV_COUNT}.nt"
: > "${NT_OUT}"
for f in "${RAW_DIR}"/*.owl; do
  # rdfxml is the actual UBA output format (file extension is .owl
  # but content is RDF/XML); rapper auto-detects.
  rapper -i rdfxml -o ntriples -q "${f}" >> "${NT_OUT}"
done

# Convert N-Triples → Turtle (compact form, useful for human review).
echo "[pgrdf-lubm-generator] converting to Turtle"
TTL_OUT="${TTL_DIR}/lubm-${UNIV_COUNT}.ttl"
rapper -i ntriples -o turtle -q "${NT_OUT}" > "${TTL_OUT}"

triples=$(wc -l < "${NT_OUT}")
size_nt=$(du -h "${NT_OUT}" | cut -f1)
size_ttl=$(du -h "${TTL_OUT}" | cut -f1)

echo "[pgrdf-lubm-generator] done"
echo "  universities:  ${UNIV_COUNT}"
echo "  triples:       ${triples}"
echo "  N-Triples:     ${NT_OUT} (${size_nt})"
echo "  Turtle:        ${TTL_OUT} (${size_ttl})"
