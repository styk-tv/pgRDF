# 07 — Identity and export

How do you know two graphs hold the same data? And how do you hand a
graph to someone so they can check they got it intact? pgRDF answers
both with digests and a portable export.

## Three digests, three questions

| Digest | Function | Method label | Answers | Conclusive when |
|---|---|---|---|---|
| **Bytes** | inside `graph_manifest(g)` | `sha256-of-canonical-ntriples` | Are these the same bytes? | equal and unequal |
| **Identity** | `graph_digest(g)` | `rdfc-1.0-sha256` | Are these the same graph? | equal and unequal |
| **Structure** | `structural_digest(g)` | `pgrdf-fd1-sha256` | Did anything structural change? | **unequal only** |

All three cover asserted triples only; inferred triples are never
included.

Values from different methods are never comparable with each other.
Always keep the method label next to a digest you store or send.

### Why bytes are not enough: blank nodes

Blank nodes get fresh labels every time data is loaded. Load the same
Turtle twice and you get the same graph with different bytes:

```sql
SELECT pgrdf.add_graph('urn:b1'), pgrdf.add_graph('urn:b2');

SELECT pgrdf.parse_turtle('@prefix ex: <http://example.org/> .
  ex:order ex:line [ ex:sku "A1" ; ex:qty 2 ] , [ ex:sku "B7" ; ex:qty 1 ] .',
  pgrdf.graph_id(g))
FROM unnest(ARRAY['urn:b1', 'urn:b2']) AS g;

SELECT g,
       pgrdf.graph_manifest(pgrdf.graph_id(g))->'digests'->'bytes'->>'value' AS bytes,
       pgrdf.graph_digest(pgrdf.graph_id(g))      AS identity,
       pgrdf.structural_digest(pgrdf.graph_id(g)) AS structure
FROM unnest(ARRAY['urn:b1', 'urn:b2']) AS g;
```

| g | bytes | identity | structure |
|---|---|---|---|
| urn:b1 | `b10d5d3e…` | `15809144…` | `6c5508b0…` |
| urn:b2 | `f706075c…` | `15809144…` | `6c5508b0…` |

The bytes differ; the identity is the same. `graph_digest` implements
the W3C **RDFC-1.0** canonicalization, which relabels blank nodes
canonically before hashing. Equal identity digests mean the graphs are
the same; unequal ones mean they differ.

For graphs without blank nodes, the bytes and identity digests are the
same value.

### When to use the structural digest

`structural_digest` is a cheaper first-degree digest (method
`pgrdf-fd1-sha256`) intended as a portable pin that other tools can
reproduce. An **unequal** result proves the graphs differ. An
**equal** result is strong evidence but not proof: certain symmetric
blank-node structures can collide, for example a cycle of four blank
nodes and two cycles of two. When you need proof, use `graph_digest`.

### Edge cases

- An empty graph has a digest: the SHA-256 of empty input
  (`e3b0c442…`).
- A graph that doesn't exist is refused with `42704`. It never
  silently hashes nothing.
- RDFC-1.0 has a complexity budget. Pathological blank-node structures
  are refused with `54000` rather than running forever.

## Exporting a graph

`pgrdf.export_graph(graph_id)` returns the graph's asserted triples as
canonical N-Triples, one triple per row, byte-sorted:

```sql
SELECT * FROM pgrdf.export_graph(pgrdf.graph_id('http://example.org/people')) LIMIT 2;
-- <http://example.org/Employee> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://xmlns.com/foaf/0.1/Person> .
-- <http://example.org/Engineer> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://example.org/Employee> .
```

Inferred triples are not exported. Run `materialize` on the imported
copy to regenerate them.

## The manifest

`pgrdf.graph_manifest(graph_id)` describes a graph and its export in one
JSON document:

```json
{
  "graph_id": 1,
  "iri": "http://example.org/people",
  "captured_at": "2026-09-10 08:07:19.478543+00",
  "engine": { "version": "0.6.34", "build_id": "v0.6.34", "extversion": "0.6.34" },
  "counts": { "asserted": 14, "inferred": 8 },
  "digests": {
    "bytes":     { "value": "c8500c48…", "method": "sha256-of-canonical-ntriples" },
    "identity":  { "value": "c8500c48…", "method": "rdfc-1.0-sha256" },
    "structure": { "value": "cc5ba8d3…", "method": "pgrdf-fd1-sha256" }
  },
  "not_carried": [
    "inferred triples (count above is a check value; re-derive with pgrdf.materialize)",
    "lifecycle state and locks (a restored copy starts unlocked and unmaterialized)",
    "attestation and provenance chains (provenance-shaped triples travel as plain triples; the proof that made them true does not)"
  ]
}
```

`not_carried` spells out what a copy of the graph does *not* include.

## Recipe: package a graph and verify it offline

Write the content and the manifest to files. The examples use the
Docker container from the [two-minute setup](../README.md#try-it-in-two-minutes).
With a local server, drop the `docker exec pgrdf` prefix.

```sh
Q="pgrdf.graph_id('http://example.org/people')"

docker exec pgrdf psql -U postgres -Atc "SELECT * FROM pgrdf.export_graph($Q)" > people.nt
docker exec pgrdf psql -U postgres -Atc "SELECT pgrdf.graph_manifest($Q)"      > people.manifest.json
```

Anyone holding the two files can check the content without a
database:

```sh
sha256sum people.nt
jq -r .digests.bytes.value people.manifest.json
# the two hashes must match
```

## Recipe: restore into a new graph and prove it's the same

N-Triples is valid Turtle, so any loader accepts the export. Restore
into a **new** graph and compare identities:

```sh
docker cp people.nt pgrdf:/tmp/people.nt
```

```sql
SELECT pgrdf.load_turtle('/tmp/people.nt',
         pgrdf.add_graph('http://example.org/people-restored'));

SELECT pgrdf.graph_digest(pgrdf.graph_id('http://example.org/people-restored'));
-- equals digests.identity.value in people.manifest.json
```

The bytes digest of a restored copy can differ if the graph contains
blank nodes, because they get new labels. The identity digest will
match.

## Next

[08 — Errors and diagnostics](08-errors-and-diagnostics.md)
