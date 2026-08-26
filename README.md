![pgRDF](docs/pgRDF-logo.png)

# pgRDF

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-18-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![CI](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml/badge.svg)](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml)
[![LATEST.md](https://img.shields.io/badge/LATEST.md-current%20advertised%20version-blue)](./LATEST.md)
[![SHACL](https://img.shields.io/badge/W3C%20SHACL%20Core-25%2F25-blue)](docs/05-validation.md)
[![Inference](https://img.shields.io/badge/inference-OWL%202%20RL%20%2B%20RDFS-success)](docs/04-inference.md)
[![Wikidata scale](https://img.shields.io/badge/scale-Wikidata%20truthy%208.2B%20triples%20ingested-blueviolet)](#scale)
[![LUBM-500](https://img.shields.io/badge/LUBM--500-112M%20quads%20materialized-blue)](#scale)

**A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning — the whole semantic stack in one database.**

## One instance instead of a farm

The usual answer to "we need a knowledge graph next to our models" is a
farm: a triple store over here, a SHACL validator service over there, a
reasoner batch job, an export pipeline between them, and glue code that
turns every question into a distributed-systems question. Each box has
its own lifecycle, its own failure modes, and its own copy of the data.

pgRDF is the other answer. One PostgreSQL extension gives you
dictionary-encoded quad storage, a SPARQL 1.1 query **and** update
engine, a W3C-conformant SHACL Core validator, an OWL 2 RL + RDFS
reasoner, canonical graph identity, and a portable export format —
**in the same database that already holds the rest of your state**.
Load RDF, then reason over it, validate it, prove what it is, package
it, and query it in place, from any client that speaks Postgres. No
sidecar store. No ETL. No second system to operate, back up, or explain
to on-call.

Scale is the ceiling, not the price of entry: a complete
8.2-billion-triple Wikidata `truthy` dump has been ingested into one
instance, and the full load → reason → query pipeline has been run end
to end at LUBM-500. The typical deployment is a right-sized graph in a
single container.

## Semantic operations

Everything below is a SQL function call inside your database.

### Query & update — SPARQL 1.1

| Surface | What ships |
|---|---|
| `pgrdf.sparql(query)` | SELECT / ASK / CONSTRUCT / DESCRIBE / UPDATE through one entry point, lowered to SQL joins on a pinned, cross-product-proof plan |
| Patterns | multi-triple `OPTIONAL`, `UNION`, `MINUS`, `VALUES`, `BIND`, named graphs (`GRAPH <iri>` and `GRAPH ?g`) composed across all of them |
| Filters | boolean composition, term-type tests, `REGEX`, numeric & typed comparison |
| Aggregates | `COUNT` / `SUM` / `AVG` / `MIN` / `MAX` / `GROUP_CONCAT` / `SAMPLE` with `GROUP BY` / `HAVING`, including over `UNION` |
| Property paths | `^` `+` `*` `?` `\|`, with a materialised-closure fast path and a depth guard |
| CONSTRUCT / DESCRIBE | graph-producing queries; DESCRIBE follows W3C §16.4 Concise Bounded Description |
| UPDATE | the complete algebra — `INSERT DATA` / `DELETE DATA` / `INSERT WHERE` / `DELETE WHERE` / `DELETE/INSERT WHERE`, graph-scoped, and fenced by graph locks like every other write path |

→ [querying guide](guide/03-querying.md) · [query engine internals](docs/03-query.md)

### Validate — W3C SHACL Core

`pgrdf.validate(data_graph, shapes_graph)` runs a genuinely conformant
SHACL Core validator — **25/25** on the W3C conformance surface —
returning a machine-readable report whose shape is frozen: tooling may
rely on it. Shapes are just another graph in the same store, so the
gate that judges your data lives beside it.

→ [validation](docs/05-validation.md)

### Reason — OWL 2 RL + RDFS

`pgrdf.materialize(graph, profile)` computes the inference closure and
stores it beside the asserted triples, never mixed into them. Inferred
triples are queryable immediately and re-derivable at any time —
derived knowledge stays derived. Each materialization records when it
ran and what it ran over, so the inventory can tell you whether it is
still current (see [Protocols](#protocols)).

→ [inference](docs/04-inference.md)

### Load — from a file to eight billion triples

| Loader | When |
|---|---|
| `parse_turtle` / `parse_trig` / `parse_nquads` | inline content, straight from SQL |
| `load_turtle` | server-side files, with lenient / verbose variants |
| `load_turtle_streaming` | windowed streaming for dumps larger than memory |
| `load_turtle_staged_run` | the multi-backend staged bulk loader — the Wikidata-scale path |

The turtle funnel records the **sha256 of the bytes it loaded**
(`source_sha256` on the graph), so downstream systems can pin exactly
what went in.

→ [loading guide](guide/02-loading-rdf.md) · [storage](docs/02-storage.md)

### Custody — graphs with a lifecycle

| Function | Effect |
|---|---|
| `add_graph` / `drop_graph` / `clear_graph` | create, remove, empty |
| `copy_graph` / `move_graph` | duplicate or rename wholesale |
| `carve_graph` | predicated sub-graph extraction — carve a working set out of a large graph |
| `lock_graph` / `unlock_graph` | write custody: a locked graph refuses **every** engine write path — SPARQL UPDATE included — until deliberately unlocked |
| `graph_inventory` / `orphan_partitions` | the supported inventory: every graph with counts, lock state, and materialization freshness; partitions nothing can reach |
| `graph_integrity` | structural health check — non-IRI predicates, literal subjects, dangling references |

→ [recipes](docs/11-recipes.md)

### Identity — two digests, two questions

A byte digest answers *"are these the same bytes"* — and blank-node
labels re-mint on every load, so two loads of one file differ in bytes
forever. Graph identity needs more than hashing, and pgRDF ships both
answers, each labelled with its method:

| Function | Method | Answers | Strength |
|---|---|---|---|
| `pgrdf.graph_digest(g)` | `rdfc-1.0-sha256` | *are these the same graph* — W3C RDFC-1.0 canonical relabelling, canonical N-Triples, sha256 | equal **and** unequal are both conclusive |
| `pgrdf.structural_digest(g)` | `pgrdf-fd1-sha256` | *did anything structural move* — first-degree blank-node signatures, the portable cross-implementation pin | unequal is conclusive; equal is **evidence, never proof** |

The asymmetry is not a caveat, it is the contract: first-degree
signing covers each node's immediate neighbourhood only, so symmetric
blank-node structures can collide (a 4-cycle and two 2-cycles of blank
nodes hash identically under fd1 — and RDFC-1.0 separates them). Where
proof is required, RDFC-1.0 is the answer; the two methods' values are
never comparable with each other. RDFC conformance is proven against
the W3C rdf-canon suite byte-for-byte.

### Package — a graph you can hand to someone

| Function | What it gives you |
|---|---|
| `pgrdf.export_graph(g)` | asserted triples as canonical N-Triples, byte-sorted, one line per triple — inferred rows never export, because derived knowledge re-derives |
| `pgrdf.graph_manifest(g)` | the portable certificate: all three digests (bytes / identity / structure) each carrying its method, counts, engine identity, capture time — and a mandatory `not_carried` list |

Together they are a redistributable package: content plus everything
needed to decide, **offline, with no database in reach**, whether a
copy is faithful — the bytes digest recomputes from the content file
with nothing but `sha256sum`. And the manifest names what a copy does
*not* carry (inferred triples, lifecycle state, attestation chains):
a copy of a graph is a transcription of its triples, and the manifest
says so instead of relying on a reader who already knows.

## Protocols

The 0.6.34 line made a doctrine explicit: **the engine emits, every
client listens.** Safety that lives in a client library serves one
language; safety that lives in the engine and travels on the wire
protocol serves every language, with nothing to install.

### Refusals are typed results

A deliberate refusal is a *result* — the engine considered the request
and declined, naming a rule — and it must never reach a client wearing
the code that means *something broke*. Every refusal gate carries its
semantic SQLSTATE:

| Class | Meaning |
|---|---|
| `55P03` | a graph lock is held |
| `55000` | wrong lifecycle state (unlock an unlocked graph, move into a non-empty destination) |
| `22023` | invalid argument or content (unknown mode/profile, negative ids, literal subjects, malformed input) |
| `0A000` | a named unsupported construct (`SERVICE`, RDF-star scopes, an untranslatable `FILTER`) |
| `42704` | unknown graph — absence refuses; an *empty* graph still answers |
| `42710` | binding conflict (an IRI or id already bound elsewhere) |
| `2BP01` | dependent objects (drop over inferred rows without `cascade`) |
| `54000` | a configured budget was exceeded under fail-closed settings |
| `XX000` | **reserved for genuine internal faults** — class XX means *no verdict was reached*; everything above means *considered and declined* |

Refusal messages remain full prose — the clause and the cure — so
`err.code` is for machines and the message is for people. The same
switch works in every driver:

```js
// node-postgres          // psycopg: e.sqlstate   // JDBC: getSQLState()
catch (e) {
  if (e.code === '55P03') …   // locked — the message names the unlock
  if (e.code === '0A000') …   // unsupported construct, named
  if (e.code === '42704') …   // no such graph
}
```

### Completeness is per-call

This engine's characteristic failure mode is an answer that is *short*,
not wrong — and a truncation that only shows up in a global counter is
unattributable under concurrency. `pgrdf.last_call_stats()` returns the
truncation and filter figures for the most recent query verb **in your
session**: another session's truncation cannot appear in them. A
truncating path also raises a `WARNING` on the statement itself, and
`SET pgrdf.on_path_truncation = 'error'` turns partial results into a
refusal outright. The cumulative `stats()` counters remain what they
always were — instance health, never a per-call verdict.

### Capability detection is a query

`pgrdf.surface()` lists every export with its stability class —
`stable` (contract), `internal`, `spike`, `deprecated` — and a
consumer-facing note. Ask the engine what it supports instead of
probing whether a function exists: existence can never tell you a
behaviour changed.

### The identity triple

A healthy deployment shows three equal values:

```sql
SELECT pgrdf.version(),                                    -- the release line
       pgrdf.build_id(),                                   -- which BUILD of it (CI-stamped from the tag)
       (SELECT extversion FROM pg_extension WHERE extname='pgrdf');
```

A workstation build self-identifies (`-dirty`, bare hash) and can never
impersonate a release; a mismatched `extversion` means an
`ALTER EXTENSION pgrdf UPDATE` is pending. Check the triple before
trusting any instance — it costs one query.

## Fail-closed, on principle

A query engine that cannot apply a clause has two options: refuse, or
silently return more than you asked for. pgRDF refuses — an
untranslatable filter raises `0A000` naming the construct, it never
silently widens a result set. The same principle runs through the
store: imports can be gated on an expected digest (a mismatch refuses
and writes nothing), locked graphs refuse writes on every path,
digest and export of an absent graph refuse rather than silently
hashing nothing, and restore-style workflows mint new graphs beside
the old rather than overwriting.

## Quickstart

Every release is a CI-built, SLSA-attested OCI artifact. Pull it, drop
two paths into a stock `postgres:18` image, done:

```sh
oras pull ghcr.io/styk-tv/pgrdf-bundle:0.6.34-pg18-amd64   # or -arm64
# → lib/pgrdf.so                → $(pg_config --pkglibdir)/
# → share/extension/pgrdf*      → $(pg_config --sharedir)/extension/
```

```sql
CREATE EXTENSION pgrdf;

SELECT pgrdf.add_graph('urn:demo');                          -- a named graph
SELECT pgrdf.parse_turtle($$
  @prefix ex: <http://example.org/> .
  ex:alice a ex:Person ; ex:knows ex:bob .
  ex:bob   a ex:Person .
$$, pgrdf.graph_id('urn:demo'));

SELECT pgrdf.sparql('SELECT ?s WHERE { ?s a <http://example.org/Person> }');
SELECT pgrdf.materialize(pgrdf.graph_id('urn:demo'));        -- OWL 2 RL closure
SELECT * FROM pgrdf.graph_inventory();                       -- counts, locks, freshness
SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:demo'));       -- canonical identity
SELECT pgrdf.graph_manifest(pgrdf.graph_id('urn:demo'));     -- the portable certificate
SELECT * FROM pgrdf.surface() WHERE class = 'stable';        -- what this engine supports
```

The current advertised release, per-architecture digests, and pull URIs
always live in [LATEST.md](./LATEST.md).

## Provenance

Releases are forward-only — one version is one commit SHA, forever — and
every published artifact carries a verifiable SLSA Build Provenance v1
attestation. Verifying is one command:

```sh
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.34 --repo styk-tv/pgRDF
```

A successful verify means: built by this repository's release workflow
from the tagged commit, signed via GitHub's Fulcio CA, recorded in
Sigstore's Rekor transparency log, digest matching what you pulled. The
full policy is [PROVENANCE.md](./PROVENANCE.md).

## Scale

Benchmarks push the limits to learn where they are:

- **Wikidata `truthy`, 8.2 billion triples**, ingested into a single
  instance through the staged bulk loader.
- **LUBM-500**: the full load → reason → query pipeline, ending in a
  112-million-quad materialised closure.

The engine that survives those runs is the same `.so` you pull above.

## Documentation

| | |
|---|---|
| [guide/](guide/) | user-facing: install, loading, querying, validation recipes |
| [docs/](docs/) | engineering: architecture, storage, query engine, inference, validation, testing, releases |
| [specs/](specs/) | authoritative specifications |
| [CHANGELOG.md](./CHANGELOG.md) | release-by-release history |
| [pgrdf.styk.tv](https://pgrdf.styk.tv) | the documentation site |

## License

[MIT](LICENSE) — © Peter Styk.
