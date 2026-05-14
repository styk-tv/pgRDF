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

| Pattern | W3C §       | Covered? |
|---|---|---|
| Basic BGP | §5 | ✅ 01 |
| DISTINCT semantics | §15.4 | ✅ 02 |
| UNION with disjoint variables (unbound → NULL) | §18.2.4 | ✅ 03 |
| Chained OPTIONAL with FILTER | §6 | ✅ 04 |
| MINUS no-shared-vars elision | §8.3.2 | ✅ 05 |
| `isIRI` term-type FILTER | §17.4.2.1 | ✅ 06 |
| `COUNT(?v)` + `GROUP BY` | §11 | ✅ 07 |
| `HAVING(?alias > c)` after SUM | §11.5 | ✅ 08 |
| `ORDER BY DESC(?v)` | §15.1 | ✅ 09 |
| `LIMIT` / `OFFSET` | §15.2 / §15.3 | ✅ 10 |
| `BIND(CONCAT(...) AS ?v)` | §10.1 + §17.4.3.2 | ✅ 11 |
| `ASK` → true | §16.2 | ✅ 12 |
| `ASK` → false | §16.2 | ✅ 13 |
| `REGEX(?v, "pat")` | §17.4.3.14 | ✅ 14 |
| `FILTER(?v IN (a, b, c))` | §17.4.1.9 | ✅ 15 |
| `STRLEN(?v)` | §17.4.3.3 | ✅ 16 |
| `LANG(?v)` | §17.4.2.4 | ✅ 17 |
| `UCASE(?v)` | §17.4.3.8 | ✅ 18 |
| `!BOUND(?v)` over OPTIONAL | §17.4.1.7 | ✅ 19 |
| `STR(?iri)` then string equality | §17.4.2.5 | ✅ 20 |
| Numeric FILTER on `xsd:integer` | §17.3 | ✅ 21 |
| Inline `HAVING(SUM(?v) > c)` | §11.5 | ✅ 22 |
| Type-aware `MIN`/`MAX` over `xsd:numeric` | §17.4 | ✅ 23 |
| Property paths beyond `:a/:b` sequence | §9 | ❌ deferred — see v0.4 |
| GRAPH `{ … }` named-graph clause | §13 | ❌ deferred — needs storage schema work |
| VALUES / FROM NAMED / CONSTRUCT / DESCRIBE | §10.2 / §13 / §16 | ❌ deferred — see v0.3 LLD §3 |

## See also

- v0.3 LLD `§5.4` Phase 6 (step 2) — `specs/SPEC.pgRDF.LLD.v0.3.md`
- v0.3 LLD coverage targets — `≥ 30 % → ≥ 70 % → ≥ 95 %`
- Roadmap — `docs/10-roadmap.md` Phase 6
