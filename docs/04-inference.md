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
- `reasoner_errors` — any `reasonable::ReasoningError` instances
  emitted during the run. Currently surfaced as `Display` strings.
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
- Regression: [`tests/regression/sql/60-materialize-owl-rl.sql`](../tests/regression/sql/60-materialize-owl-rl.sql)
- ERRATA: [`E-002`](../specs/ERRATA.v0.2.md) — narrows the LLD §2
  reference from "Datalog reasoner" to "OWL 2 RL".
