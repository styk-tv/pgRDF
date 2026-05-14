#!/usr/bin/env bash
#
# Re-generate fixtures/regression/synth-10k.ttl: 10 000 deterministic
# triples. Used by `52-bulk-ingest-perf.sql` to verify the prepared-
# INSERT plan cache (Phase 3 step 3, LLD §4.3 phase A) fires across
# many batches per load.
#
# Output structure (1000-batch-friendly):
#   - 100 distinct subjects (ex:s0 .. ex:s99), each in 100 triples
#   - 10 distinct predicates (ex:p0 .. ex:p9), each in 1000 triples
#   - 10 000 distinct objects (ex:o<i>_<j>) — all unique
#
# Total distinct terms: 100 + 10 + 10 000 = 10 110.
# Total term references: 10 000 × 3 = 30 000.
# Expected hashmap hit count: 30 000 − 10 110 = 19 890.

set -euo pipefail
out="$(cd "$(dirname "$0")" && pwd)/synth-10k.ttl"

{
    cat <<'HEADER'
# Deterministic 10 000-triple fixture. Re-generate with
# `bash fixtures/regression/synth-10k.sh` if you change the structure.

@prefix ex: <http://example.com/synth/> .

HEADER
    for i in $(seq 0 99); do
        for j in $(seq 0 99); do
            p=$((j % 10))
            printf 'ex:s%d ex:p%d ex:o%d_%d .\n' "$i" "$p" "$i" "$j"
        done
    done
} > "${out}"

echo "wrote ${out} ($(wc -l < "${out}") lines)"
