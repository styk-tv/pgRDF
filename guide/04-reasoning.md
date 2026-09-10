# 04 — Reasoning

`pgrdf.materialize()` computes what your data implies and stores the
result next to it. Inferred triples are queryable right away and never
mixed into your asserted data.

```text
pgrdf.materialize(graph_id bigint, profile text DEFAULT 'owl-rl') → jsonb
```

## Profiles

| Profile | What it derives |
|---|---|
| `'owl-rl'` (default) | The OWL 2 RL rule set: RDFS entailment plus `owl:sameAs`, `owl:inverseOf`, transitive / symmetric / functional / inverse-functional properties, and the rest of the RL profile, except property chains (`owl:propertyChainAxiom` is not entailed). |
| `'rdfs'` | RDFS entailment only: `rdfs:subClassOf`, `rdfs:subPropertyOf`, `rdfs:domain`, `rdfs:range`. A strict subset of `'owl-rl'`. |

Any other profile name is refused with SQLSTATE `22023`:

```sql
SELECT pgrdf.materialize(1, 'bogus');
-- ERROR:  22023: materialize: unknown profile "bogus" (supported: 'owl-rl', 'rdfs')
```

OWL 2 EL and QL profiles, and custom rule languages, are not supported.

## Example

```sql
SELECT pgrdf.add_graph('http://example.org/zoo');
SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix owl:  <http://www.w3.org/2002/07/owl#> .

ex:Sparrow    rdfs:subClassOf ex:Bird .
ex:Bird       rdfs:subClassOf ex:Animal .
ex:parentOf   owl:inverseOf   ex:childOf .
ex:ancestorOf a owl:TransitiveProperty .

ex:jack a ex:Sparrow ; ex:parentOf ex:jill .
ex:ann  ex:ancestorOf ex:bea . ex:bea ex:ancestorOf ex:cal .
$$, pgrdf.graph_id('http://example.org/zoo'));

SELECT pgrdf.materialize(pgrdf.graph_id('http://example.org/zoo'));
```

Afterwards the graph also contains, among others:

- `ex:jack a ex:Bird`, `ex:jack a ex:Animal` (subclass),
- `ex:jill ex:childOf ex:jack` (inverse),
- `ex:ann ex:ancestorOf ex:cal` (transitive).

The OWL 2 RL profile also adds a small, fixed set of axiomatic triples
(for example `rdf:type rdf:type rdf:Property`), even for data with no
schema at all.

## The result report

`materialize` returns a JSONB report:

```json
{
  "profile": "owl-rl",
  "base_triples": 14,
  "inferred_triples_written": 18,
  "previous_inferred_dropped": 8,
  "reasoner_errors": [],
  "auto_analyzed": true,
  "elapsed_ms": 1.24
}
```

It also includes a per-phase timing breakdown (`load_ms`, `reason_ms`,
`diff_ms`, `write_ms`, `analyze_ms`).

| Field | Meaning |
|---|---|
| `base_triples` | Asserted triples the reasoner read. |
| `inferred_triples_written` | New triples it derived. |
| `previous_inferred_dropped` | Inferred triples from the previous run that were replaced. |
| `auto_analyzed` | Planner statistics were refreshed afterwards (setting `pgrdf.auto_analyze`, on by default). |

## Asserted vs inferred

Every graph keeps its two kinds of triple apart:

| | Asserted | Inferred |
|---|---|---|
| Seen by `sparql` / `construct` / `describe` | yes | yes |
| Seen by `validate` | yes | yes |
| Counted in `graph_inventory()` | `asserted` column | `inferred` column |
| Included in `export_graph` and the digests | yes | **no**, they can always be re-derived |
| Carried by `copy_graph` | yes | yes |

Running `materialize` again replaces the previous inferred set with a
fresh one, so it is safe to repeat.

## Is the materialization current?

`graph_inventory()` reports a `materialization` state for every graph:

| State | Meaning |
|---|---|
| `never` | Never materialized and holds no inferred triples. |
| `current` | Materialized, and the asserted data has not changed size since. |
| `stale` | The asserted data changed after the last run. Run `materialize` again. |
| `unknown` | Holds inferred triples whose origin was not recorded, for example rows carried in by `copy_graph`. |

```sql
SELECT iri, asserted, inferred, materialization
  FROM pgrdf.graph_inventory()
 WHERE materialization IN ('stale', 'unknown');
```

Freshness is tracked by the asserted triple count. An edit that
removes one triple and adds another leaves the count unchanged and
still reads `current`. If you edit in place, re-materialize.

## Dropping a graph with inferred triples

`drop_graph` removes inferred rows along with the graph by default.
Pass `cascade => false` to make it refuse instead:

```sql
SELECT pgrdf.drop_graph(pgrdf.graph_id('http://example.org/zoo'), false);
-- ERROR:  2BP01: drop_graph: inferred rows present (graph_id = 1); pass cascade => true to proceed
```

## Performance notes

- The reasoner runs inside the database process, over one graph at a
  time. Memory use grows with the size of the graph.
- Transitive property paths such as `rdfs:subClassOf+` automatically
  use the materialized closure when it is present, and skip the
  recursive walk.
- The full load → reason → query pipeline has been run at LUBM-500
  scale: a 112-million-quad materialized closure in one instance.

## Next

[05 — Managing graphs](05-graphs.md)
