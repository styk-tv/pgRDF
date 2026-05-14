# tests/w3c-sparql

Hand-authored W3C-shape SPARQL conformance tests. Each subdirectory
is one test:

```
01-basic-bgp/
  data.ttl       — Turtle input loaded into a fresh graph
  query.rq       — SPARQL query executed via pgrdf.sparql
  expected.jsonl — one JSONB result row per line, lexicographically sorted
  description.md — optional prose explaining the W3C spec section exercised
```

**Phase 6 step 2 (v0.3) ships the runner + 5 starter tests.** The
**actual W3C SPARQL 1.1 manifest runner** — reading
`https://w3c.github.io/rdf-tests/sparql/sparql11/manifest.ttl`,
materialising each test from `mf:QueryEvaluationTest`, comparing
SRX / SRJ result graphs — is a v0.4 work item; we don't yet
parse SRX/SRJ.

## Runner

```bash
bash tests/w3c-sparql/run.sh                  # all tests
bash tests/w3c-sparql/run.sh 01-basic-bgp     # one test
ACCEPT=1 bash tests/w3c-sparql/run.sh         # regenerate expected.jsonl
```

The runner:
1. `DROP EXTENSION IF EXISTS pgrdf CASCADE; CREATE EXTENSION pgrdf;`
   (fresh dictionary each test — no cross-test pollution).
2. Picks a graph id from the test name so tests stay isolated.
3. Loads `data.ttl` via `pgrdf.parse_turtle`.
4. Runs `query.rq` via `pgrdf.sparql`.
5. Sorts both expected and actual JSONL lexicographically (so the
   comparison is bag-equivalent — SPARQL solution sequences are
   un-ordered unless ORDER BY is present).
6. `diff -u` reports any mismatch.

## Adding a new test

```bash
mkdir tests/w3c-sparql/06-new-name
$EDITOR tests/w3c-sparql/06-new-name/data.ttl
$EDITOR tests/w3c-sparql/06-new-name/query.rq
ACCEPT=1 bash tests/w3c-sparql/run.sh 06-new-name   # write expected.jsonl
# Hand-verify the output against the W3C spec — never trust
# ACCEPT=1 blind.
git add tests/w3c-sparql/06-new-name
```

`ACCEPT=1` captures what the engine emits today; verifying it
against the W3C spec is the load-bearing part.

## Scope

| Pattern | Covered? |
|---|---|
| Basic BGP | ✅ 01 |
| DISTINCT semantics | ✅ 02 |
| UNION with disjoint variables (unbound → NULL) | ✅ 03 |
| Chained OPTIONAL with filter | ✅ 04 |
| MINUS no-shared-vars elision | ✅ 05 |
| Property paths beyond `:a/:b` sequence | ❌ deferred — see v0.4 |
| GRAPH `{ … }` named-graph clause | ❌ deferred — needs storage schema work |
| VALUES / FROM NAMED / CONSTRUCT / DESCRIBE | ❌ deferred — see v0.3 LLD §3 |

## See also

- v0.3 LLD `§5.4` Phase 6 (step 2) — `specs/SPEC.pgRDF.LLD.v0.3.md`
- v0.3 LLD coverage targets — `≥ 30 % → ≥ 70 % → ≥ 95 %`
- Roadmap — `docs/10-roadmap.md` Phase 6
