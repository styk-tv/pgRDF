# 04 — Inference

> **Status: shipped (Phase 4).** `pgrdf.materialize(graph_id)` is live;
> see [`src/inference/reasonable.rs`](../src/inference/reasonable.rs)
> and regression `60-materialize-owl-rl.sql`.

`SELECT pgrdf.materialize(<graph_id>) → JSONB`. Reads every
`is_inferred = FALSE` quad in the named graph, hands them to the
`reasonable` crate (OWL 2 RL forward-chain reasoner), and writes
entailed-but-not-asserted triples back into the same partition with
`is_inferred = TRUE`.

## Scope (per ERRATA E-002)

`reasonable` is an **OWL 2 RL** reasoner — **not full OWL 2** and
**not arbitrary Datalog**. Concretely:

- ✅ `rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`
- ✅ `owl:sameAs`, `owl:inverseOf`, `owl:TransitiveProperty`
- ✅ `owl:SymmetricProperty`, `owl:FunctionalProperty` /
      `owl:InverseFunctionalProperty`
- ✅ Property chains (`owl:propertyChainAxiom`) — within RL bounds
- ❌ OWL 2 EL / QL profiles (no `owl:hasSelf`, no role-composition
      beyond the RL allowance, …)
- ❌ Custom Datalog rules
- ❌ DL-Lite-style query rewriting

If a TBox needs EL (SNOMED-style) or QL, the slice would need a
different reasoner (e.g. ELK or a custom DL engine). Tracked in
[`docs/10-roadmap.md`](10-roadmap.md) v0.4+.

## Reasoning-profile selector (✅ Phase G group G1)

```sql
pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT 'owl-rl') → JSONB
```

The bare `pgrdf.materialize(g)` form is **unchanged** — it defaults
`profile => 'owl-rl'` and is behaviourally identical to the v0.3 /
v0.4 surface. v0.5 adds the profile selector
([LLD v0.5 §3](../specs/SPEC.pgRDF.LLD.v0.5.md), shipped in v0.5.0):

| Profile | Behaviour |
|---|---|
| `'owl-rl'` (default) | Full OWL 2 RL forward-chain via `reasonable` (the existing path, unchanged). |
| `'rdfs'` | The RDFS entailment-rule subset only — a strict, sound, complete RDFS forward-chain (rdfs2/3/5/7/9/11). A true subset of `'owl-rl'`. |
| any other string | Errors `materialize: unknown profile …` — **no silent fallback**. The reserved future `'owl-rl-ext'` is treated as unknown until a later cycle wires it. |

The JSONB stats object gains a `profile` field reflecting the
requested profile. Why route 2 (a pgRDF-internal RDFS engine, not
upstream profile support): the patched `reasonable` fork exposes
only a fused OWL-RL fixpoint, so pgRDF computes the RDFS closure
itself — restricted to the six productive RDFS rules so it stays a
true subset of OWL-RL (the §3.1 subset + agreement criteria hold by
construction). Full rationale in
[LLD v0.5 §3.2](../specs/SPEC.pgRDF.LLD.v0.5.md).

```sql
-- RDFS-only entailment (e.g. for an RDFS-scoped workload class):
SELECT pgrdf.materialize(g, 'rdfs');
-- → {"base_triples":7,"inferred_triples_written":6,"profile":"rdfs",…}

-- Full OWL 2 RL (default — identical to pre-v0.5 pgrdf.materialize(g)):
SELECT pgrdf.materialize(g, 'owl-rl');
SELECT pgrdf.materialize(g);            -- same thing

-- Unknown profile → error, not a fallback:
SELECT pgrdf.materialize(g, 'bogus');   -- ERROR: materialize: unknown profile "bogus" …
```

## Flow

```text
  SELECT pgrdf.materialize(g)
         │
         ▼  DELETE _pgrdf_quads WHERE graph_id = g AND is_inferred = TRUE
         │     (count → previous_inferred_dropped in the JSONB stats)
         ▼
  SELECT … _pgrdf_quads JOIN _pgrdf_dictionary × 3 + LEFT JOIN dt
         │   one round-trip; each row rehydrates into an oxrdf::Triple
         ▼
  Reasoner::new().load_triples(base).reason()
         │   forward chain to fixpoint over the RL rule set
         ▼  reasoner.get_triples()
         │   set-diff against the base HashSet<Triple>
         ▼  inferred-only Vec<&Triple>
         │   put_term_full(s/p/o) intern (shmem-warm path)
         ▼
  INSERT INTO _pgrdf_quads (… , is_inferred = TRUE)
```

## Stats surface

```json
{
  "base_triples":              123,
  "inferred_triples_written":  45,
  "previous_inferred_dropped": 42,
  "profile":                   "owl-rl",
  "reasoner_errors":           [],
  "elapsed_ms":                17.4
}
```

- `base_triples` — count of `is_inferred = FALSE` quads passed in.
- `inferred_triples_written` — set-difference between
  `reasoner.get_triples()` and the input set, i.e. entailed-but-
  not-asserted triples written this call.
- `previous_inferred_dropped` — rows wiped before this run (= the
  previous run's `inferred_triples_written` if you call back-to-back).
- `profile` — the requested reasoning profile (`'owl-rl'` for the
  default-arg call, or `'rdfs'`). Phase G group G1.
- `reasoner_errors` — any `reasonable::ReasoningError` instances
  emitted during the run. Currently surfaced as `Display` strings.
  (The `'rdfs'` profile is pure pgRDF code — always `[]`.)
- `elapsed_ms` — wall clock for the whole UDF.

## Idempotency

- `materialize(g)` first deletes every `is_inferred = TRUE` row in
  graph `g`, then re-derives from scratch. Two back-to-back calls
  produce the same row count; the second `previous_inferred_dropped`
  equals the first `inferred_triples_written`.
- Pure-data graphs (no axioms) still emit a small constant set of
  OWL 2 RL **axiomatic triples** (e.g. `rdf:type rdf:type
  rdf:Property`). The base graph survives unchanged — verified by
  the `materialize_pure_data_preserves_input` pgrx test.

## Performance notes

- Base rehydration is one SPI scan + three dictionary JOINs. For a
  10 000-triple graph that's a single ~100 ms round-trip on the
  shipped indexes.
- The reasoner is in-process and CPU-bound; expect roughly linear
  scaling in graph size for typical OWL 2 RL ontologies.
- Writeback is currently row-by-row INSERT (one SPI call per
  inferred triple). For graphs with many entailments this is the
  hotspot; switching to the cached `INSERT … unnest` flush path
  from `src/storage/loader.rs::flush_batch` is the obvious v0.4
  follow-up and is tracked as a Phase 3 step 3b dependency.

## Removing inferred state without re-deriving

```sql
DELETE FROM pgrdf._pgrdf_quads
 WHERE graph_id = $1 AND is_inferred = TRUE;
```

Fast under partition-pruning. The base graph is preserved. The next
`materialize` call re-derives.

## See also

- Implementation: [`src/inference/reasonable.rs`](../src/inference/reasonable.rs)
- Regressions:
  [`60-materialize-owl-rl.sql`](../tests/regression/sql/60-materialize-owl-rl.sql) (core OWL 2 RL entailments + idempotence + inverseOf),
  [`61-materialize-then-sparql.sql`](../tests/regression/sql/61-materialize-then-sparql.sql) (inferred triples flow through `pgrdf.sparql`),
  [`62-materialize-empty.sql`](../tests/regression/sql/62-materialize-empty.sql) (zero-triple edge case),
  [`117-materialize-rdfs.sql`](../tests/regression/sql/117-materialize-rdfs.sql) (the `'rdfs'` profile: subset of owl-rl + RDFS-axiom agreement + unknown-profile error + no-CTE compose — Phase G group G1).
- ERRATA: [`E-002`](../specs/ERRATA.v0.2.md) — narrows the LLD §2
  reference from "Datalog reasoner" to "OWL 2 RL".
- Reasoning profile selector: shipped Phase G group G1, released
  in v0.5.0 —
  [`specs/SPEC.pgRDF.LLD.v0.5.md`](../specs/SPEC.pgRDF.LLD.v0.5.md) §3 / §3.2
  (route + precise `'rdfs'` semantics).
