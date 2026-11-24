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
included. All three describe the graph as a **set** of triples: see
[digests describe the set of triples](#digests-describe-the-set-of-triples).

Values from different methods are never comparable with each other.
Always keep the method label next to a digest you store or send.

### Why bytes are not enough: blank nodes

Anonymous blank nodes (`[ … ]`) get fresh labels every time data is
loaded. Load the same Turtle twice and you get the same graph with
different bytes:

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
same value. Labelled blank nodes (`_:b1`) keep the label the document
gave them, so two loads of such a document also agree on bytes; rely on
the identity digest all the same, because a copy made by another tool
need not keep the labels.

### When to use the structural digest

`structural_digest` is a cheaper first-degree digest (method
`pgrdf-fd1-sha256`) intended as a portable pin that other tools can
reproduce. An **unequal** result proves the graphs differ. An
**equal** result is strong evidence but not proof: certain symmetric
blank-node structures can collide, for example a cycle of four blank
nodes and two cycles of two. When you need proof, use `graph_digest`.

### Locked graphs remember their digest

A locked graph cannot change, so its `graph_digest` is computed once
and then served from the stored value (visible as `identity_digest` in
`graph_inventory()`). Unlocking clears it. Open graphs are digested on
every call, so the answer is always current.

### Digests describe the set of triples

Since 0.6.41 `graph_digest`, `structural_digest`, the manifest's bytes
digest and `export_graph` depend only on which triples a graph holds,
not on how many times a triple is stored. Every graph written by
0.6.41 is a set anyway. The difference shows on a graph that an
earlier version left holding repeated triples (see
[repeated triples from earlier versions](05-graphs.md#repeated-triples-from-earlier-versions)):
it now digests and exports exactly like the same graph without the
repeats.

What that means for values you already hold:

- **A graph without repeats: nothing changes.** Every digest and every
  export line is byte-identical to what 0.6.40 produced.
- **A graph that held repeats when you recorded its digest:** the old
  value counted the repeated triples as stored, and the value 0.6.41
  computes does not, so the two differ. `pgrdf.duplicate_report()`
  names the graphs this applies to. The old value is not wrong about
  what was stored then; it is a digest of a different thing, and it
  cannot be reproduced by 0.6.41. Record the new one.
- **A locked graph keeps answering the digest recorded for it**, even
  if an earlier version computed it over repeats. The value is a
  recorded fact and stays until the graph is unlocked.
  `duplicate_report()` shows in `locked_digest_engine` which pgRDF
  version computed it; empty means none since 0.6.41. Unlock, run
  `dedup_graph`, and lock again to have the digest of the set
  recorded.

`graph_inventory()` also shows `source_sha256`, the SHA-256 of the file a
graph was loaded from. That identifies the *artifact*, not the graph: the
same triples loaded from a Turtle file and from an N-Triples file have
different source digests and the same `graph_digest`. Compare like with
like: source digests with source digests, graph digests with graph digests.

## What changed between two graphs

`graph_diff_summary(a, b)` and `graph_diff(a, b)` compare two graphs
and are exact for graphs with blank nodes, which a `MINUS` query is
not: every load gives blank nodes new internal ids, so a triple-by-
triple comparison reports each blank-node triple as both removed and
added.

```sql
SELECT pgrdf.graph_diff_summary(pgrdf.graph_id('http://example.org/v1'),
                                pgrdf.graph_id('http://example.org/v2'));
-- {"method": "ground set difference + blank-node components by RDFC-1.0 component digest; …",
--  "ground": {"removed": 2, "added": 1, "common": 1102763},
--  "components": {"removed": 1, "added": 1, "common": 37, "triples_removed": 3, "triples_added": 3},
--  "by_predicate": [{"p": "http://www.w3.org/ns/shacl#minCount", "removed": 1, "added": 1}, …],
--  "by_type": [{"class": "http://example.org/C", "removed": 0, "added": 1}]}
```

- **Ground triples** (no blank node) are compared exactly, one by one.
- **Blank-node components** are the triples connected through shared
  blank nodes: one RDF list, one SHACL property shape. Each is
  canonicalised on its own (RDFC-1.0) and compared as a whole, so a
  changed `sh:minCount` shows up as that property shape removed and
  added, never as a renumbered mess.
- The diff is empty exactly when the two graphs are isomorphic, the
  same test `graph_digest` makes.
- Counts: `ground.*` and `components.triples_*` are triples;
  `components.removed/added/common` are components; `by_predicate`
  covers both kinds of triples; `by_type` counts `rdf:type` triples per
  class.

`graph_diff` returns the triples themselves, `side` `-` (only in `a`)
or `+` (only in `b`), as N-Triples terms, with `component` set to the
component's digest for blank-node triples (their labels print as
`_:<digest>_c14n0`, stable across calls). Rows come in a fixed order,
so `OFFSET` / `LIMIT` paging is deterministic, and three optional
arguments narrow the result: `side`, `predicate` (an IRI) and `class`
(an `rdf:type` object). A component is returned whole when any of its
triples matches.

```sql
SELECT side, subject, object
  FROM pgrdf.graph_diff(1, 2, class => 'http://example.org/C');
```

Memory is bounded by `pgrdf.diff_max_rows` (default 5,000,000); past
it the call is refused with `54000`.

### Edge cases

- An empty graph has a digest: the SHA-256 of empty input
  (`e3b0c442…`).
- A graph that doesn't exist is refused with `42704`. It never
  silently hashes nothing.
- RDFC-1.0 has a complexity budget. Pathological blank-node structures
  are refused rather than running forever, with the message
  `canonicalization budget exceeded`.

### Size and memory

`export_graph`, `graph_manifest`, `graph_digest` and
`structural_digest` read every asserted triple of a graph. Since 0.6.41
they stream: PostgreSQL sorts the lines, on disk once they exceed
`work_mem`, and pgRDF reads them a batch at a time, so the memory of
the database process no longer grows with the graph.

One part is still held in memory: the triples that involve a blank
node, which the two digests need together to label the blank nodes.
`pgrdf.digest_max_bnode_quads` (default 5,000,000) bounds how many;
past it `graph_digest`, `structural_digest` and `graph_manifest` are
refused with `54000`:

```sql
SET pgrdf.digest_max_bnode_quads = 1;
SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:b1'));
-- ERROR:  54000: graph_digest: graph 1 holds more than 1 distinct triples that involve a
--         blank node (pgrdf.digest_max_bnode_quads=1)
-- HINT:  Blank-node labelling needs those triples in memory together; ground triples are
--        streamed and do not count. Raise pgrdf.digest_max_bnode_quads deliberately, with
--        the memory to hold them.
```

Triples without blank nodes do not count towards it, and
`export_graph` has no such limit.

### A stored U+0000

Versions before 0.6.41 could store a term containing the character
U+0000, which PostgreSQL `text` cannot hold; reading such a term back
failed or cut it short. Loading now [refuses it](02-loading-rdf.md#characters-that-cannot-be-stored).
A graph that still holds one is refused by `export_graph`,
`graph_manifest`, `graph_digest` and `structural_digest` with `22021`
before any line is returned, so no cut-off export leaves the database.
The message names the term's dictionary id and the graph, and the
hint the cure: reload the graph from a source with the character
removed or replaced.

`pgrdf.nul_term_report()` lists every such term with the graphs that
use it. No rows means there are none:

```sql
SELECT * FROM pgrdf.nul_term_report();
--  term_id | term_type | preview | graph_ids
-- ---------+-----------+---------+-----------
-- (0 rows)
```

`term_type` is `1` for an IRI, `2` for a blank node, `3` for a literal;
`preview` shows the start of the term with the character written
`\u0000`. The function reads the whole dictionary, so run it as an
occasional check, not per request.

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
  "captured_at": "2026-11-24 14:15:16.21078+00",
  "consistent": false,
  "engine": { "version": "0.6.41", "build_id": "v0.6.41", "extversion": "0.6.41" },
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

Every value describes the set of asserted triples:

- `counts.asserted` is the number of lines `export_graph` returns.
- `counts.repeats` appears only for a graph that stores some triple
  more than once, and is the number of surplus rows. A graph written by
  0.6.41 never has it.
- `counts.inferred` is a check value; inferred triples are not
  exported.

`consistent` says whether the three digests are known to describe the
same state. They are three separate reads of the graph, so a write
that lands between them could make them disagree. `consistent` is
`true` when that cannot have happened: the graph is locked, or the
transaction runs at `REPEATABLE READ` or stricter. When it is `false`
and the graph may be written while you read, take the manifest again
like this:

```sql
BEGIN ISOLATION LEVEL REPEATABLE READ;
SELECT pgrdf.graph_manifest(pgrdf.graph_id('http://example.org/people')) -> 'consistent';
-- true
COMMIT;
```

For a package, export and manifest belong in one such transaction (or
take them from a locked graph), so the file and its manifest describe
the same state.

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
