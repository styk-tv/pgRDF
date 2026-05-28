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

cd /opt/lubm/uba1.7

# The UBA generator wants `classes/` on the classpath and the
# Univ-Bench ontology as a file:// URI for the -onto argument.
java -cp classes \
  edu.lehigh.swat.bench.uba.Generator \
  -univ "${UNIV_COUNT}" \
  -onto "file:///opt/lubm/uba1.7/univ-bench.daml" \
  -timestamp 0 \
  -seed 0

# UBA writes outputs to its working directory; move them into the
# volume's raw subdir.
mv University*_*.owl "${RAW_DIR}/"

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
