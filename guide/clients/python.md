# Python

pgRDF is plain SQL, so any PostgreSQL driver works. Examples below use
the Docker setup from the [install guide](../01-install.md)
(`postgres` / `pgrdf` on `localhost:5432`).

## psycopg 3

```bash
pip install "psycopg[binary]>=3.2"
```

```python
import psycopg

DSN = "postgresql://postgres:pgrdf@localhost:5432/postgres"

TURTLE = """
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice foaf:name "Alice" ; foaf:age 34 ; foaf:knows ex:bob .
ex:bob   foaf:name "Bob"   ; foaf:age 41 .
"""

with psycopg.connect(DSN) as conn:
    cur = conn.cursor()
    cur.execute("CREATE EXTENSION IF NOT EXISTS pgrdf")

    # create a graph and load Turtle passed as a parameter
    cur.execute("SELECT pgrdf.add_graph(%s)", ("http://example.org/people",))
    (graph_id,) = cur.fetchone()
    cur.execute("SELECT pgrdf.parse_turtle(%s, %s)", (TURTLE, graph_id))
    print("loaded", cur.fetchone()[0], "triples")

    # SPARQL: each row is one JSONB object, adapted to a dict
    cur.execute("""
        SELECT sparql FROM pgrdf.sparql(%s)
    """, ("""
        PREFIX foaf: <http://xmlns.com/foaf/0.1/>
        SELECT ?name ?age WHERE { ?p foaf:name ?name ; foaf:age ?age }
        ORDER BY ?name
    """,))
    for (row,) in cur:
        print(row["name"], int(row["age"]))     # values arrive as strings
```

JSONB results come back as Python `dict`s.

### Handling refusals

Refusals carry a SQLSTATE, and psycopg maps each one to an exception
class:

```python
from psycopg import errors

try:
    cur.execute("SELECT pgrdf.materialize(%s, %s)", (graph_id, "owl-rl"))
except errors.LockNotAvailable as e:        # 55P03: graph is locked
    print("locked:", e)
except psycopg.Error as e:
    print(e.sqlstate, e)                     # e.g. 22023, 0A000, 42704
```

### Was the answer complete?

Read the per-call figures on the same connection, right after the query:

```python
cur.execute("SELECT pgrdf.last_call_stats()")
stats = cur.fetchone()[0]
complete = stats["path_depth_truncations"] == 0 and stats["filter_clauses_dropped"] == 0
```

## asyncpg

```bash
pip install "asyncpg>=0.30"
```

asyncpg returns JSONB as text unless you register a codec:

```python
import asyncio, json
import asyncpg

async def main():
    conn = await asyncpg.connect("postgresql://postgres:pgrdf@localhost/postgres")
    await conn.set_type_codec("jsonb", encoder=json.dumps, decoder=json.loads,
                              schema="pg_catalog")
    try:
        gid = await conn.fetchval("SELECT pgrdf.add_graph($1)", "http://example.org/people")
        await conn.fetchval("SELECT pgrdf.parse_turtle($1, $2)",
                            '<http://example.org/a> <http://example.org/p> "x" .', gid)
        rows = await conn.fetch(
            "SELECT sparql FROM pgrdf.sparql($1)",
            "SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o }")
        for r in rows:
            print(r["sparql"]["s"], r["sparql"]["o"])
    except asyncpg.exceptions.LockNotAvailableError as e:
        print("locked:", e)
    finally:
        await conn.close()

asyncio.run(main())
```

## SQLAlchemy

```python
from sqlalchemy import create_engine, text

engine = create_engine("postgresql+psycopg://postgres:pgrdf@localhost/postgres")

with engine.begin() as conn:
    rows = conn.execute(
        text("SELECT sparql FROM pgrdf.sparql(:q)"),
        {"q": "SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 5"},
    )
    for (binding,) in rows:
        print(binding)
```

## Working with rdflib

rdflib can parse and serialize many formats client-side. Serialize to
N-Triples and pass the text to pgRDF:

```python
from rdflib import Graph

g = Graph().parse("ontology.rdf")                 # RDF/XML, JSON-LD, ...
cur.execute("SELECT pgrdf.parse_turtle(%s, %s)", (g.serialize(format="nt"), graph_id))
```

## Tips

- `pgrdf.load_turtle(path, …)` reads from the **database server's**
  filesystem. For files on the client, read them in Python and use
  `parse_turtle`.
- Build SPARQL with parameters for the SQL call (`%s`), but remember
  that the SPARQL text itself is a string. Escape any user input you
  splice into it.
- `SET search_path = pgrdf, public` lets you drop the `pgrdf.` prefix.
