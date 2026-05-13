# Python clients

pgRDF exposes its capabilities as SQL UDFs, so any standard Postgres
client library works. This page covers the two most common —
`psycopg` (sync) and `asyncpg` (async) — plus a sketch of using
pgRDF as a backend for `rdflib` if your codebase is already invested
in that ecosystem.

## psycopg 3

```bash
pip install "psycopg[binary]" >= 3.2
```

```python
import psycopg

with psycopg.connect("postgresql://pgrdf:pgrdf@localhost:5432/pgrdf") as conn:
    with conn.cursor() as cur:
        # First-time setup (or use a migration tool).
        cur.execute("CREATE EXTENSION IF NOT EXISTS pgrdf")

        # Load a Turtle file from the server-side filesystem.
        cur.execute(
            "SELECT pgrdf.load_turtle(%s, %s)",
            ("/fixtures/ontologies/foaf.ttl", 1),
        )
        n_triples = cur.fetchone()[0]
        print(f"loaded {n_triples} triples")

        # Parse an in-memory Turtle string.
        cur.execute(
            "SELECT pgrdf.parse_turtle(%s, %s)",
            (
                "@prefix ex: <http://example.com/> . ex:a ex:p ex:b .",
                2,
            ),
        )

        # See structured ingest stats via the verbose variant.
        cur.execute(
            "SELECT pgrdf.load_turtle_verbose(%s, %s, %s)",
            (
                "/fixtures/ontologies/prov.ttl",
                100,
                "http://www.w3.org/ns/prov#",
            ),
        )
        (stats,) = cur.fetchone()    # → dict
        print(f"prov.ttl: {stats['triples']} triples in {stats['elapsed_ms']:.0f}ms")

    conn.commit()
```

Note: `pgrdf.load_turtle_verbose` returns `JSONB`; psycopg adapts
that to a Python `dict` by default.

## asyncpg

```bash
pip install asyncpg >= 0.30
```

```python
import asyncio
import asyncpg

async def main():
    conn = await asyncpg.connect("postgresql://pgrdf:pgrdf@localhost:5432/pgrdf")
    try:
        await conn.execute("CREATE EXTENSION IF NOT EXISTS pgrdf")

        n = await conn.fetchval(
            "SELECT pgrdf.load_turtle($1, $2)",
            "/fixtures/ontologies/foaf.ttl", 1,
        )
        print(f"loaded {n} triples")

        # JSONB stats come back as a dict.
        stats = await conn.fetchval(
            "SELECT pgrdf.load_turtle_verbose($1, $2, $3)",
            "/fixtures/ontologies/prov.ttl", 100, "http://www.w3.org/ns/prov#",
        )
        print(stats["triples"], "triples in", stats["elapsed_ms"], "ms")

        # Quad-count by graph
        n_in_g1 = await conn.fetchval("SELECT pgrdf.count_quads($1)", 1)
        print(f"graph 1 holds {n_in_g1} quads")
    finally:
        await conn.close()

asyncio.run(main())
```

## SQLAlchemy

If you already use SQLAlchemy, you can wire pgRDF as plain text
SQL through the regular session. Map the JSONB return from the
`*_verbose` UDFs to `sqlalchemy.dialects.postgresql.JSONB` so it
deserialises to a `dict`:

```python
from sqlalchemy import create_engine, text
from sqlalchemy.dialects.postgresql import JSONB

engine = create_engine("postgresql+psycopg://pgrdf:pgrdf@localhost/pgrdf")

with engine.begin() as conn:
    n = conn.scalar(
        text("SELECT pgrdf.load_turtle(:path, :graph)"),
        {"path": "/fixtures/ontologies/foaf.ttl", "graph": 1},
    )
    print(f"loaded {n} triples")
```

## rdflib bridge (sketch — Phase 2.2 step 5)

`rdflib` is the dominant Python RDF library. Today, its default
`Memory` and `BerkeleyDB` stores keep triples client-side. A natural
position for pgRDF is as an `rdflib.store.Store` implementation that
delegates `add` / `remove` / `triples` to pgRDF UDFs over a regular
psycopg connection.

The shape (once `pgrdf.sparql(q)` lands):

```python
# Conceptual — full implementation arrives with the SPARQL surface.
from rdflib import Graph
from pgrdf_rdflib import PgRDFStore     # to-be-shipped sibling project

store = PgRDFStore(dsn="postgresql://pgrdf:pgrdf@localhost/pgrdf", graph_id=1)
g = Graph(store=store)

g.parse("foaf.ttl", format="turtle")    # delegates to pgrdf.parse_turtle
list(g.triples((None, RDF.type, FOAF.Person)))   # delegates to a server-side BGP query
```

Until then, you can use rdflib client-side to parse + manipulate
graphs and pgRDF server-side for storage + bulk ops — they don't
collide.

## Caveats

- `load_turtle` reads the path from the postgres process. Your
  application's working directory is irrelevant.
- pgRDF strictness: any Turtle that fails to load is genuinely
  off-spec. Don't paper over parse errors in client code — fix the
  TTL.
- The extension's schema is `pgrdf`. Set
  `search_path = pgrdf, public` once per session if you want to
  drop the `pgrdf.` prefix on every call.
