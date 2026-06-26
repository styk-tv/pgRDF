# pgRDF regression suite — `pg_regress`-style golden tests

Each `sql/NN-<topic>.sql` is piped to `psql` against a freshly-installed
extension; its stdout is diffed against the matching `expected/NN-<topic>.out`.
Any unexpected diff fails CI. **95 tests, 95 goldens, 1:1 — no orphans, no
un-baselined files.**

```
tests/regression/
├── run.sh            # the driver: glob sql/*.sql → psql → diff vs expected/
├── sql/      NN-*.sql # the STIMULUS — the test script (setup + fixtures + queries)
├── expected/ NN-*.out # the ORACLE   — golden stdout it must reproduce (tuples-only)
└── scripts/          # two orchestration tests that can't run inside psql -c
    ├── pg-dump-roundtrip.sh
    └── verify-installed-artifacts.sh
```

`sql/` vs `expected/` is a **role** split, not a membership split: the `.sql`
is the input (1–22 KB), the `.out` is the captured result (4 B–1.5 KB — small
because `run.sh` runs psql with `-t -A`, so the golden is just the bare result
rows).

## Running

| Command | Scope |
|---|---|
| `just test-regression` | all 93 (`bash tests/regression/run.sh`) |
| `bash tests/regression/run.sh 71-shacl-real` | one test by exact basename |
| `just test-regression-accept` / `ACCEPT=1 … run.sh` | re-baseline `expected/` from actual |
| `just test-pg-dump-roundtrip` | `scripts/pg-dump-roundtrip.sh` (separate) |
| `just verify-installed-artifacts` | `scripts/verify-installed-artifacts.sh` (separate) |

`run.sh` takes **all** or **one exact name** today — there is no group / range /
tag selection yet. The cross-cutting **sweeps** below are the *map* of what to
run when you touch a subsystem; wiring them into `run.sh` is the
[Regression Roadmap](#regression-roadmap).

> There is **no metadata file** — the suite is convention-over-configuration.
> The number drives run order; each file's header comment is its spec. This
> index is hand-maintained from those headers.

## Sweeps at a glance

A **sweep** is a capability-named, cross-cutting set of tests — the second axis
over the linear file numbering (a test has exactly one number but can belong to
one capability). Several sweeps span non-contiguous numbers (`dict`, `caching`,
`ingest-paths`), which is exactly why number-ranges alone can't express them.

| Sweep | Tests | Span | Proves |
|---|---:|---|---|
| [`smoke`](#smoke) | 1 | 00 | extension installs + version answers |
| [`storage`](#storage) | 4 | 11–12, 72, 82 | quad/partition primitives + on-disk shape contracts |
| [`dict`](#dict) | 3 | 10, 123–124 | dictionary term round-trip + exact-lexical-byte preservation |
| [`ingest`](#ingest) | 10 | 20–25, 52, 65, 119–120 | Turtle/N-Quads/TriG loaders + parsers across all term shapes |
| [`ingest-paths`](#ingest-paths) | 5 | 128–133 | `ingest_dict_path` route dispatch + spike-path parity gates |
| [`sparql-core`](#sparql-core) | 23 | 30–44, 66, 100ᵒᵇ, 112–116, 121 | SELECT/ASK algebra surface end-to-end |
| [`construct`](#construct) | 8 | 100–107 | `pgrdf.construct` templates + round-trip |
| [`property-paths`](#property-paths) | 4 | 108–111 | `^ + * ?` paths + materialised-closure elision |
| [`named-graphs`](#named-graphs) | 8 | 73–79, 87 | `GRAPH {}` query surface + IRI↔id mapping UDFs |
| [`graph-lifecycle`](#graph-lifecycle) | 6 | 88–92, 118 | drop/clear/copy/move + integration + IRI overloads |
| [`sparql-update`](#sparql-update) | 7 | 93–99 | INSERT/DELETE DATA/WHERE + lifecycle algebra |
| [`inference`](#inference) | 6 | 60–62, 117, 134–135 | `materialize` OWL-RL + RDFS profiles + type-closure inclusion/exclusion patterns |
| [`validation`](#validation) | 3 | 70–71, 122 | SHACL Core + SHACL-SPARQL modes |
| [`caching`](#caching) | 4 | 50–51, 63–64 | shmem dict cache + per-backend plan cache |
| [`contracts`](#contracts) | 3 | 80–81, 127 | stable error prefixes + failure modes + search-path discipline |

Total: **95**. Every test belongs to exactly one sweep above.

## The index

Grouped by sweep; numeric order within each. Each row links the stimulus and
its oracle.

### smoke
| # | Test | Verifies |
|---|---|---|
| 00 | [smoke](sql/00-smoke.sql) · [out](expected/00-smoke.out) | `CREATE EXTENSION pgrdf` installs cleanly; `pgrdf.version()` answers; idempotent across fresh-PGDATA + re-runs |

### storage
| # | Test | Verifies |
|---|---|---|
| 11 | [quads-basic](sql/11-quads-basic.sql) · [out](expected/11-quads-basic.out) | `put_quad` + `count_quads` on the default partition |
| 12 | [graphs](sql/12-graphs.sql) · [out](expected/12-graphs.out) | `add_graph` creates a named partition; `put_quad` routes by `graph_id` |
| 72 | [graphs-table-shape](sql/72-graphs-table-shape.sql) · [out](expected/72-graphs-table-shape.out) | `_pgrdf_graphs` IRI↔id table shape — columns/types, PK + UNIQUE, seed row `(0,'urn:pgrdf:graph:0')`, no leaks |
| 82 | [stats-shape](sql/82-stats-shape.sql) · [out](expected/82-stats-shape.out) | `pgrdf.stats()` JSONB field set + types + non-negativity (operator-facing contract) |

### dict
| # | Test | Verifies |
|---|---|---|
| 10 | [dict-roundtrip](sql/10-dict-roundtrip.sql) · [out](expected/10-dict-roundtrip.out) | `put_term` / `get_term` + dedup semantics (BEGIN/ROLLBACK-isolated) |
| 123 | [dictionary-lexical-contract](sql/123-dictionary-lexical-contract.sql) · [out](expected/123-dictionary-lexical-contract.out) | every term shape round-trips EXACT lexical bytes (no trim / no zero-strip; lang lowercased per RDF 1.1) |
| 124 | [end-to-end-lexical-rehydration](sql/124-end-to-end-lexical-rehydration.sql) · [out](expected/124-end-to-end-lexical-rehydration.out) | exact lexicals survive the full parse→construct→materialize→validate pipeline |

### ingest
| # | Test | Verifies |
|---|---|---|
| 20 | [load-turtle](sql/20-load-turtle.sql) · [out](expected/20-load-turtle.out) | `load_turtle` on the committed 5-triple FOAF fixture (stable counts) |
| 21 | [typed-literals](sql/21-typed-literals.sql) · [out](expected/21-typed-literals.out) | XSD datatypes parsed; datatype IRIs interned as dict URIs |
| 22 | [lang-tags](sql/22-lang-tags.sql) · [out](expected/22-lang-tags.out) | language-tagged literals dedup per (value, lang) |
| 23 | [blank-nodes](sql/23-blank-nodes.sql) · [out](expected/23-blank-nodes.out) | `[]` Turtle syntax desugars to one bnode + three triples |
| 24 | [rdf-list](sql/24-rdf-list.sql) · [out](expected/24-rdf-list.out) | `( 1 2 3 )` desugars to an rdf:first/rest chain ending in rdf:nil |
| 25 | [bulk-ingest](sql/25-bulk-ingest.sql) · [out](expected/25-bulk-ingest.out) | dict-cache + batched quad INSERT on the 100-triple synth fixture (hit arithmetic) |
| 52 | [bulk-ingest-perf](sql/52-bulk-ingest-perf.sql) · [out](expected/52-bulk-ingest-perf.out) | batched-INSERT plan prepared once + reused per backend (plan-cache counters, not wall-clock) |
| 65 | [parse-turtle-empty](sql/65-parse-turtle-empty.sql) · [out](expected/65-parse-turtle-empty.out) | triple-free input (empty / whitespace / comment / `@prefix`-only) returns 0; no rows, no panic |
| 119 | [parse-nquads](sql/119-parse-nquads.sql) · [out](expected/119-parse-nquads.out) | 4-position N-Quads routing + 3-position default fallback; `strict` rejects unknown graph IRI |
| 120 | [parse-trig](sql/120-parse-trig.sql) · [out](expected/120-parse-trig.out) | TriG inline `GRAPH {}` blocks load N graphs in one call; per-graph round-trip |

### ingest-paths
| # | Test | Verifies |
|---|---|---|
| 128 | [parse-turtle-dict-batched-parity](sql/128-parse-turtle-dict-batched-parity.sql) · [out](expected/128-parse-turtle-dict-batched-parity.out) | 2-pass batched dict path ≡ baseline `parse_turtle` (decoded triples match) |
| 129 | [shmem-cache-prewarm](sql/129-shmem-cache-prewarm.sql) · [out](expected/129-shmem-cache-prewarm.out) | `shmem_cache_prewarm(limit)` walks the dict into shmem correctly (reset-isolated) — *also touches `caching`* |
| 130 | [ingest-dict-paths-parity](sql/130-ingest-dict-paths-parity.sql) · [out](expected/130-ingest-dict-paths-parity.out) | 4 `ingest_dict_path` routes (baseline/batched/shmem_warm/combined) produce identical quads; bad GUC → combined |
| 132 | [quad-dict-paths-parity](sql/132-quad-dict-paths-parity.sql) · [out](expected/132-quad-dict-paths-parity.out) | same 4-path parity for `parse_nquads` + `parse_trig` quad-stream ingest |
| 133 | [verbose-path-field](sql/133-verbose-path-field.sql) · [out](expected/133-verbose-path-field.out) | verbose JSONB echoes the selected `ingest_dict_path` route; default + unknown-value fallback locked |

### sparql-core
| # | Test | Verifies |
|---|---|---|
| 30 | [sparql-parse](sql/30-sparql-parse.sql) · [out](expected/30-sparql-parse.out) | `sparql_parse` returns a stable JSONB shape for SELECT |
| 31 | [sparql-bgp](sql/31-sparql-bgp.sql) · [out](expected/31-sparql-bgp.out) | single-pattern BGP → SQL + variable binding + JSONB output |
| 32 | [sparql-multipattern](sql/32-sparql-multipattern.sql) · [out](expected/32-sparql-multipattern.out) | N-pattern BGP where shared variables become INNER joins |
| 33 | [sparql-filter](sql/33-sparql-filter.sql) · [out](expected/33-sparql-filter.out) | FILTER identity, boolean composition, term-type predicates, BOUND |
| 34 | [sparql-filter-advanced](sql/34-sparql-filter-advanced.sql) · [out](expected/34-sparql-filter-advanced.out) | numeric ordering, REGEX, IN |
| 35 | [sparql-modifiers](sql/35-sparql-modifiers.sql) · [out](expected/35-sparql-modifiers.out) | DISTINCT / REDUCED, ORDER BY (ASC/DESC), LIMIT, OFFSET |
| 36 | [sparql-optional](sql/36-sparql-optional.sql) · [out](expected/36-sparql-optional.out) | OPTIONAL → LEFT JOIN (single-triple; chaining; ON-clause FILTER) |
| 37 | [sparql-union](sql/37-sparql-union.sql) · [out](expected/37-sparql-union.out) | UNION → UNION ALL with NULL-padded branch shapes |
| 38 | [sparql-minus](sql/38-sparql-minus.sql) · [out](expected/38-sparql-minus.out) | MINUS → WHERE NOT EXISTS keyed on shared variables |
| 39 | [sparql-aggregates](sql/39-sparql-aggregates.sql) · [out](expected/39-sparql-aggregates.out) | COUNT / COUNT(DISTINCT) / SUM / AVG / MIN / MAX + GROUP BY |
| 40 | [sparql-having](sql/40-sparql-having.sql) · [out](expected/40-sparql-having.out) | HAVING (post-aggregate filter) + GROUP_CONCAT + SAMPLE |
| 41 | [sparql-expressions](sql/41-sparql-expressions.sql) · [out](expected/41-sparql-expressions.out) | arithmetic + string fns (STRLEN/CONTAINS/UCASE/LCASE/LANG/DATATYPE…) |
| 42 | [sparql-bind](sql/42-sparql-bind.sql) · [out](expected/42-sparql-bind.out) | `BIND(expr AS ?v)` projection (locks v0.3 projection-only behaviour) |
| 43 | [sparql-minus-multi](sql/43-sparql-minus-multi.sql) · [out](expected/43-sparql-minus-multi.out) | multi-triple MINUS (whole-subpattern AND, not any-triple) |
| 44 | [sparql-ask](sql/44-sparql-ask.sql) · [out](expected/44-sparql-ask.out) | ASK → single-row `{"_ask": "true"/"false"}` |
| 66 | [parse-sparql-roundtrip](sql/66-parse-sparql-roundtrip.sql) · [out](expected/66-parse-sparql-roundtrip.out) | parse_turtle→sparql visibility across all 4 object kinds + bnode subject |
| 100ᵒᵇ | [sparql-order-by-type-aware](sql/100-sparql-order-by-type-aware.sql) · [out](expected/100-sparql-order-by-type-aware.out) | type-aware ORDER BY by value space (numeric/dateTime/boolean/string); DESC; multi-key |
| 112 | [optional-multi-triple](sql/112-optional-multi-triple.sql) · [out](expected/112-optional-multi-triple.out) | N-triple OPTIONAL binds atomically (all-or-nothing); nesting; inner FILTER |
| 113 | [values-inline](sql/113-values-inline.sql) · [out](expected/113-values-inline.out) | VALUES inline tables incl. `UNDEF`; joined to the surrounding BGP |
| 114 | [bind-downstream](sql/114-bind-downstream.sql) · [out](expected/114-bind-downstream.out) | BIND var visible to a later FILTER / join / chained BIND (v0.4 §11) |
| 115 | [aggregate-over-union](sql/115-aggregate-over-union.sql) · [out](expected/115-aggregate-over-union.out) | aggregates / GROUP BY / HAVING over UNION branches |
| 116 | [describe](sql/116-describe.sql) · [out](expected/116-describe.out) | `pgrdf.describe` resource closure incl. transitive blank-node one-hop |
| 121 | [agg-union-residual](sql/121-agg-union-residual.sql) · [out](expected/121-agg-union-residual.out) | the 6 deferred aggregate-over-UNION cases now answer correctly (no panic) |

### construct
| # | Test | Verifies |
|---|---|---|
| 100 | [construct-foundation](sql/100-construct-foundation.sql) · [out](expected/100-construct-foundation.out) | `pgrdf.construct` constant-only templates; structured-term row shape; reject non-CONSTRUCT/literal-subject |
| 101 | [construct-variable-templates](sql/101-construct-variable-templates.sql) · [out](expected/101-construct-variable-templates.out) | variable substitution in subject / predicate / object positions |
| 102 | [construct-blank-node-templates](sql/102-construct-blank-node-templates.sql) · [out](expected/102-construct-blank-node-templates.out) | fresh-per-solution bnode labels; within-solution sameness |
| 103 | [construct-multi-triple-templates](sql/103-construct-multi-triple-templates.sql) · [out](expected/103-construct-multi-triple-templates.out) | N-triple templates; N×M cardinality; cross-triple bnode joining within a solution |
| 104 | [construct-graph-scoped-where](sql/104-construct-graph-scoped-where.sql) · [out](expected/104-construct-graph-scoped-where.out) | `GRAPH <iri>` / `GRAPH ?g` WHERE scoping; `?g` projected from the graph IRI |
| 105 | [construct-where-shorthand](sql/105-construct-where-shorthand.sql) · [out](expected/105-construct-where-shorthand.out) | `CONSTRUCT WHERE {}` shorthand; BGP-only + no-blank-node restrictions |
| 106 | [construct-round-trip](sql/106-construct-round-trip.sql) · [out](expected/106-construct-round-trip.out) | `put_construct_rows` re-ingest preserves graph state (bnode joining, idempotent) |
| 107 | [sparql-parse-construct](sql/107-sparql-parse-construct.sql) · [out](expected/107-sparql-parse-construct.out) | `sparql_parse` CONSTRUCT shape (template / where_shape / shorthand) contract |

### property-paths
| # | Test | Verifies |
|---|---|---|
| 108 | [property-path-inverse](sql/108-property-path-inverse.sql) · [out](expected/108-property-path-inverse.out) | `^` inverse (+ nested fold); shared BGP walker; `path_max_depth` GUC bounds |
| 109 | [property-path-plus](sql/109-property-path-plus.sql) · [out](expected/109-property-path-plus.out) | `+` one-or-more via recursive CTE; cycle-safe; depth-guard truncation counter |
| 110 | [property-path-star-opt](sql/110-property-path-star-opt.sql) · [out](expected/110-property-path-star-opt.out) | `*` zero-or-more + `?` zero-or-one with the W3C §9.3 zero-length node-set |
| 111 | [property-path-materialised-closure](sql/111-property-path-materialised-closure.sql) · [out](expected/111-property-path-materialised-closure.out) | materialised closure elides the recursive CTE (EXPLAIN-scraped), identical results |

### named-graphs
| # | Test | Verifies |
|---|---|---|
| 73 | [add-graph-populates-iri](sql/73-add-graph-populates-iri.sql) · [out](expected/73-add-graph-populates-iri.out) | `add_graph(id)` seeds a synthetic `urn:pgrdf:graph:{id}` row; idempotent |
| 74 | [add-graph-iri](sql/74-add-graph-iri.sql) · [out](expected/74-add-graph-iri.out) | `add_graph(iri)→id` auto-allocates next id; idempotent on IRI; binds verbatim |
| 75 | [add-graph-id-iri](sql/75-add-graph-id-iri.sql) · [out](expected/75-add-graph-id-iri.out) | `add_graph(id,iri)` explicit binding; conflict errors; synthetic-placeholder UPDATE upgrade |
| 76 | [graph-id-lookup](sql/76-graph-id-lookup.sql) · [out](expected/76-graph-id-lookup.out) | `graph_id(iri)→id` (NULL on miss; STRICT) |
| 77 | [graph-iri-lookup](sql/77-graph-iri-lookup.sql) · [out](expected/77-graph-iri-lookup.out) | `graph_iri(id)→iri`, inverse of 76; round-trip |
| 78 | [sparql-graph-literal-iri](sql/78-sparql-graph-literal-iri.sql) · [out](expected/78-sparql-graph-literal-iri.out) | `GRAPH <iri> {}` scoping; unresolved IRI → 0 rows, no error |
| 79 | [sparql-graph-variable](sql/79-sparql-graph-variable.sql) · [out](expected/79-sparql-graph-variable.out) | `GRAPH ?g {}` INNER-joins `_pgrdf_graphs`; `?g` projected as IRI |
| 87 | [sparql-graph-composition](sql/87-sparql-graph-composition.sql) · [out](expected/87-sparql-graph-composition.out) | per-pattern GRAPH scope composed with OPTIONAL / UNION / MINUS |

### graph-lifecycle
| # | Test | Verifies |
|---|---|---|
| 88 | [drop-graph](sql/88-drop-graph.sql) · [out](expected/88-drop-graph.out) | `drop_graph(id,cascade)` detaches+drops partition; cascade / default / negative guards |
| 89 | [clear-graph](sql/89-clear-graph.sql) · [out](expected/89-clear-graph.out) | `clear_graph(id)` TRUNCATEs the partition but keeps the binding; `clear(0)` allowed |
| 90 | [copy-graph](sql/90-copy-graph.sql) · [out](expected/90-copy-graph.out) | `copy_graph(src,dst)` INSERT…SELECT; auto-creates dst; preserves `is_inferred` |
| 91 | [move-graph](sql/91-move-graph.sql) · [out](expected/91-move-graph.out) | `move_graph` = copy+drop compose; dst-has-data + self-move + negative guards |
| 92 | [lifecycle-end-to-end](sql/92-lifecycle-end-to-end.sql) · [out](expected/92-lifecycle-end-to-end.out) | drop/clear/copy/move wired together over a realistic load→mutate→verify flow |
| 118 | [lifecycle-iri-overloads](sql/118-lifecycle-iri-overloads.sql) · [out](expected/118-lifecycle-iri-overloads.out) | IRI-keyed drop/clear/copy/move; unbound IRI **errors** (vs the BIGINT no-op) |

### sparql-update
| # | Test | Verifies |
|---|---|---|
| 93 | [update-insert-data](sql/93-update-insert-data.sql) · [out](expected/93-update-insert-data.out) | `INSERT DATA` (default + named graph); `{_update}` summary; set-semantic idempotent |
| 94 | [update-delete-data](sql/94-update-delete-data.sql) · [out](expected/94-update-delete-data.out) | `DELETE DATA` lookup-only; no-op on missing term; named-graph scope |
| 95 | [update-insert-where](sql/95-update-insert-where.sql) · [out](expected/95-update-insert-where.out) | `INSERT { tmpl } WHERE { pat }` per-solution; unbound-var + variable-GRAPH rejects |
| 96 | [update-delete-where](sql/96-update-delete-where.sql) · [out](expected/96-update-delete-where.out) | `DELETE { tmpl } WHERE { pat }`; counts actual rows removed; FILTER-narrowed |
| 97 | [update-delete-insert-where](sql/97-update-delete-insert-where.sql) · [out](expected/97-update-delete-insert-where.out) | combined DELETE+INSERT WHERE over one WHERE snapshot (DELETE-first atomicity) |
| 98 | [update-graph-scoped](sql/98-update-graph-scoped.sql) · [out](expected/98-update-graph-scoped.out) | GRAPH-scoped variants + `WITH <g>` desugar across template + WHERE |
| 99 | [update-lifecycle-algebra](sql/99-update-lifecycle-algebra.sql) · [out](expected/99-update-lifecycle-algebra.out) | DROP/CLEAR/CREATE GRAPH + DEFAULT/ALL/NAMED + SILENT semantics |

### inference
| # | Test | Verifies |
|---|---|---|
| 60 | [materialize-owl-rl](sql/60-materialize-owl-rl.sql) · [out](expected/60-materialize-owl-rl.out) | `materialize` OWL 2 RL forward-chain (subClassOf chain, inverseOf, idempotent) |
| 61 | [materialize-then-sparql](sql/61-materialize-then-sparql.sql) · [out](expected/61-materialize-then-sparql.out) | inferred triples visible to a subsequent `pgrdf.sparql` (engine composition) |
| 62 | [materialize-empty](sql/62-materialize-empty.sql) · [out](expected/62-materialize-empty.out) | `materialize` on an empty graph — no panic, well-formed stats, idempotent |
| 117 | [materialize-rdfs](sql/117-materialize-rdfs.sql) · [out](expected/117-materialize-rdfs.out) | `materialize(g,'rdfs')` profile (rdfs2/3/5/7/9/11 subset); unknown-profile error |
| 134 | [wikidata-type-closure-materialise](sql/134-wikidata-type-closure-materialise.sql) · [out](expected/134-wikidata-type-closure-materialise.out) | K-7 (§8): `wdt:P279 a owl:TransitiveProperty` + `materialize` → subclass closure as direct edges; plain `?s wdt:P31 ?t . ?t wdt:P279 X` recovers subclass-typed instances (depth-cap-free) |
| 135 | [type-closure-lubm-patterns](sql/135-type-closure-lubm-patterns.sql) · [out](expected/135-type-closure-lubm-patterns.out) | carve groundwork over the real LUBM hierarchy (tracked `.nt` fixture): direct anchor omits subclass-typed entities; `materialize` → plain `rdf:type` inclusion + `MINUS` exclusion, both complete (`FILTER NOT EXISTS` / `MINUS`-with-path are unsupported) |

### validation
| # | Test | Verifies |
|---|---|---|
| 70 | [validate-stub](sql/70-validate-stub.sql) · [out](expected/70-validate-stub.out) | `validate(data,shapes)` emits a real W3C `sh:ValidationReport` JSONB shape |
| 71 | [shacl-real](sql/71-shacl-real.sql) · [out](expected/71-shacl-real.out) | SHACL Core — sh:NodeShape + sh:property + sh:datatype violations on missing props |
| 122 | [shacl-modes](sql/122-shacl-modes.sql) · [out](expected/122-shacl-modes.out) | `validate(data_id,shapes_id,mode)` native/sparql modes; unknown-mode error; over a materialized graph |

### caching
| # | Test | Verifies |
|---|---|---|
| 50 | [shmem-dict-cache](sql/50-shmem-dict-cache.sql) · [out](expected/50-shmem-dict-cache.out) | cold→hot shmem dict cache across loads (hand-computed hit/insert deltas) |
| 51 | [plan-cache](sql/51-plan-cache.sql) · [out](expected/51-plan-cache.out) | structurally-identical SPARQL reuses a prepared statement (hit/miss deltas) |
| 63 | [shmem-reset-invalidation](sql/63-shmem-reset-invalidation.sql) · [out](expected/63-shmem-reset-invalidation.out) | `shmem_reset()` bumps the generation → stale entries read cold |
| 64 | [plan-cache-clear](sql/64-plan-cache-clear.sql) · [out](expected/64-plan-cache-clear.out) | `plan_cache_clear()` returns the drained count; idempotent at zero |

### contracts
| # | Test | Verifies |
|---|---|---|
| 80 | [unsupported-shapes](sql/80-unsupported-shapes.sql) · [out](expected/80-unsupported-shapes.out) | unsupported SPARQL shapes fail with stable error substrings (never silent wrong results) |
| 81 | [error-paths](sql/81-error-paths.sql) · [out](expected/81-error-paths.out) | stable Rust-side error prefixes — missing file / bad id / malformed Turtle / bad query |
| 127 | [search-path-discipline](sql/127-search-path-discipline.sql) · [out](expected/127-search-path-discipline.out) | `#[search_path(pgrdf,pg_temp)]` resists schema-shadow attacks + session-path gaps |

### scripts/ (orchestration tests, run separately)
These boot/inspect the compose stack across multiple `psql` + binary
invocations, so they live outside the `sql/`+`expected/` golden model and are
**not** driven by `run.sh`.

| Script | Run via | Verifies |
|---|---|---|
| [pg-dump-roundtrip.sh](scripts/pg-dump-roundtrip.sh) | `just test-pg-dump-roundtrip` | `pg_dump` of a pgRDF DB round-trips the `_pgrdf_graphs` IRI mapping verbatim (LLD v0.4 §3.1) |
| [verify-installed-artifacts.sh](scripts/verify-installed-artifacts.sh) | `just verify-installed-artifacts` (and CI) | the running container has THIS repo's extension bytes mounted (fresh-export byte-compare + version surface) |

## Authoring a new regression test

1. Pick the next free `NN` (gaps are fine — forward-only, never re-use a number).
2. Write `sql/<NN>-<topic>.sql`. Open with a header comment whose **first line**
   is `<NN>-<topic> — <one-line description>`; this index harvests that line.
   Hand-compute expected values in the header — never `ACCEPT=1`-baseline a test
   whose output you haven't reasoned out.
3. Run `bash tests/regression/run.sh <NN>-<topic>`. With no `expected/` file it
   prints `BASELINE` and writes the `.out`.
4. **Inspect the captured output.** If correct, keep it; commit both files. The
   diff in the PR is the audit trail.
5. Add a row to the [index](#the-index) under its sweep, and to the matching
   `[sweep.*]` entry once `sweeps.toml` lands (see roadmap).

When a test legitimately changes (you fixed a bug and the output must update),
re-accept the same way; the PR diff documents it.

---

# Regression Roadmap

> Design notes for systematising the suite. None of this is wired yet — it
> captures the target so the index above stays the single source of truth in the
> meantime.

## 1. Make sweeps runnable (`sweeps.toml` + `run.sh`)

The index already *defines* the sweeps; the next step is a machine-readable
manifest so `run.sh` can trigger them. Proposed `tests/regression/sweeps.toml`
— the only new metadata file, generated-from / kept-in-sync-with this index:

```toml
# tests/regression/sweeps.toml — capability sweeps over the linear sql/ numbering.
# A sweep names a cross-cutting set of tests; run.sh resolves @name → file list.
# Ranges are inclusive by leading number; explicit basenames disambiguate the
# 100-collision (see §3).

[sweep.sparql-core]
description = "SELECT/ASK algebra: BGP, FILTER, modifiers, OPTIONAL/UNION/MINUS, aggregates, BIND, VALUES, DESCRIBE, ORDER BY."
tests = ["30-44", "66", "100-sparql-order-by-type-aware", "112-116", "121"]

[sweep.construct]
description = "pgrdf.construct templates + round-trip."
tests = ["100-construct-foundation", "101-107"]

[sweep.property-paths]
description = "^ inverse, + * ? recursion, materialised-closure elision."
tests = ["108-111"]

# … one block per sweep in the at-a-glance table …

[sweep.caching]
description = "shmem dict cache + per-backend plan cache."
tests = ["50-51", "63-64"]
also = ["129-shmem-cache-prewarm"]   # cross-cutting: primary sweep is ingest-paths
```

Triggering model — extend `run.sh`'s single-arg matcher to:

| Invocation | Selects |
|---|---|
| `run.sh` | all 93 (unchanged) |
| `run.sh 71-shacl-real` | one test (unchanged) |
| `run.sh @construct` | the `construct` sweep |
| `run.sh @construct @property-paths` | the union of two sweeps |
| `run.sh 30-44` | a numeric range |
| `run.sh 1xx` | a numeric prefix (all 100-series) |

**Relative vs all.** *Relative* = run the sweep(s) covering the subsystem you
touched (edited the CONSTRUCT emitter → `run.sh @construct @sparql-core`; touched
the dict path → `run.sh @ingest @ingest-paths @caching`) for a fast inner loop.
*All* = the full 93 gate before merge / release. CI keeps running **all**; sweeps
are a developer-velocity tool, not a way to ship partial coverage — so
`sweeps.toml` should carry a CI assertion that `⋃ sweeps == sql/*.sql` (no test
escapes a sweep).

A test can carry a primary sweep + `also` tags (e.g. 129 is `ingest-paths`
primary but logically `caching`); `run.sh @caching` would include `also` members.

## 2. Fix the stale band convention

The current README documents number bands `00-09 smoke … 80-99 validation`. The
suite outgrew them: the `100–133` range (CONSTRUCT, property paths,
OPTIONAL/VALUES/BIND, N-Quads/TriG, SHACL modes, dict-path parity) has no band,
and several bands no longer match content (e.g. `82-stats-shape` is storage-shape,
not query; `80-81` are contract tests, not validation). **Resolution:** retire the
band table as the organising principle; keep numbers as *run-order + identity
only*, and let **sweeps** be the semantic axis (this README already does that).
Document "numbers are monotonic ids, not categories" so nobody re-derives a
category from a number.

## 3. Resolve the `100-` number collision

Two tests share number 100: `100-construct-foundation` and
`100-sparql-order-by-type-aware`. They run fine (lexical order: `construct` <
`sparql`), but it breaks the "NN is unique" assumption that range selection
(`run.sh 100`) and tooling rely on. **Options:** (a) renumber the order-by test
to the next free slot (e.g. `126`) — clean but rewrites a committed test's
identity; (b) accept the collision and require explicit basenames for both in
`sweeps.toml` (no range can name them unambiguously). Recommend (a) at the next
natural touch of that file; until then, **never** select `100` by bare number.

## 4. Coverage gaps to fill

Genuine thin/absent spots surfaced by building the index:

- **Staged / streaming loaders untested here.** `load_turtle_staged_run` (and the
  streaming/windowed paths the at-scale benchmark exercises) have **no** golden in
  this suite — they're only covered by the out-of-tree benchmark harness. A
  small-fixture staged-vs-baseline parity test (in the spirit of `130`) belongs in
  `ingest-paths`.
- **Property-path `|` alternation + negated property sets** are preview-panic only
  (108–110 lock the *failure* message). When E4 ships they need positive golden
  coverage; track as `property-paths` debt.
- **Concurrency / parallel ingest correctness** has no regression — the parallel
  dict + concurrent id-reservation paths are validated only by hand + benchmarks.
- **DESCRIBE** has one test (116); the bnode-cycle + `DESCRIBE *` edges are inside
  it but multi-resource closures could use a dedicated case.
- **W3C conformance + LUBM are separate suites** (`tests/w3c-sparql/`,
  `tests/w3c-shacl/`, `tests/perf/lubm-shape/`, run via `just test-conformance`).
  Worth a top-level pointer so the regression suite isn't mistaken for total
  coverage; a `conformance` meta-sweep could chain them.

## 5. Keep the index honest

The index is hand-maintained from header first-lines today. Once `sweeps.toml`
exists, a tiny generator (`make-index.sh`: read each `sql/*.sql` header line 1 +
its sweep membership → emit the markdown tables) removes the drift risk that
already bit the band convention. The header-comment format (`NN-topic — desc` on
line 1) is the contract that makes this mechanical — authoring step 2 above keeps
it parseable.
