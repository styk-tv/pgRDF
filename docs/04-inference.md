# 04 — Inference

> **Status: planned — not yet implemented.** This doc describes the
> Phase 4 design. No `pgrdf.materialize` UDF exists today; the
> `is_inferred` column on `_pgrdf_quads` is wired but never written
> to. Tracked in [`docs/10-roadmap.md`](10-roadmap.md) Phase 4.

Triggered by `SELECT pgrdf.materialize(<graph_id>)`. Reads quads from
the named graph, runs them through `reasonable` (OWL 2 RL), and
materializes inferred triples back into the same partition with
`is_inferred = TRUE`.

## Scope (per ERRATA E-002)

`reasonable` is an **OWL 2 RL** reasoner — **not full OWL 2** and
**not arbitrary Datalog**. Concretely:

- ✅ `rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`
- ✅ `owl:sameAs`, `owl:inverseOf`, `owl:TransitiveProperty`
- ✅ `owl:SymmetricProperty`, `owl:FunctionalProperty`
- ❌ OWL 2 EL/QL profiles
- ❌ Custom Datalog rules

If your TBox needs the EL profile (e.g. SNOMED-style ontologies), you
will need to extend Phase 3+ with a different reasoner; see
`docs/10-roadmap.md` for the rough scope of that work.

## Flow

```
SELECT pgrdf.materialize(g)
       │
       ▼
stream _pgrdf_quads WHERE graph_id = g  ─► reasonable
                                               │
                                               ▼  iterate to fixpoint
                                          inferred triples
                                               │
                                               ▼
COPY _pgrdf_quads (.., is_inferred=true) FROM STDIN BINARY
```

## Idempotency

- Repeated `materialize(g)` is **safe** but **not free**: the loader
  computes the inferred set from scratch each time, then upserts (no
  duplicates are written thanks to the hexastore covering indexes).
- For incremental materialization (only re-derive what changes when
  a small delta is added), see Phase 3.

## Removing inferred state

```sql
DELETE FROM _pgrdf_quads WHERE graph_id = $1 AND is_inferred = TRUE;
```

This is fast under partition-pruning. The base graph is preserved.
