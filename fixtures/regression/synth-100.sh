#!/usr/bin/env bash
#
# Re-generate fixtures/regression/synth-100.ttl: 100 deterministic
# triples with predictable term repetition for testing the dict cache
# and batched-INSERT paths in src/storage/loader.rs.
#
# Output structure:
#   - 10 distinct subjects (ex:s0 .. ex:s9), each appearing in 10
#     triples → cache hits 9 out of every 10 references.
#   - 5 distinct predicates (ex:p0 .. ex:p4) → each appears 20 times.
#   - 100 distinct objects (ex:o<i>_<j>) — all unique, never repeat.
#
# Total distinct terms: 10 + 5 + 100 = 115.
# Total term references: 100 × 3 = 300.
# Expected cache hit count: 300 − 115 = 185.

set -euo pipefail
out="$(cd "$(dirname "$0")" && pwd)/synth-100.ttl"

{
    cat <<'HEADER'
# Deterministic 100-triple fixture. Re-generate with
# `bash fixtures/regression/synth-100.sh` if you change the structure;
# do not hand-edit unless you also update tests/regression/sql/25.

@prefix ex: <http://example.com/synth/> .

HEADER
    for i in $(seq 0 9); do
        for j in $(seq 0 9); do
            p=$((j % 5))
            printf 'ex:s%d ex:p%d ex:o%d_%d .\n' "$i" "$p" "$i" "$j"
        done
    done
} > "${out}"

echo "wrote ${out} ($(wc -l < "${out}") lines)"
