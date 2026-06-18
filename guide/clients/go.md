# Go clients

`pgx` is the canonical Go driver for Postgres and integrates with
pgRDF identically to any other extension — every capability is a
SQL function call.

## pgx (v5)

```bash
go get github.com/jackc/pgx/v5
```

```go
package main

import (
	"context"
	"fmt"
	"log"

	"github.com/jackc/pgx/v5"
)

func main() {
	ctx := context.Background()
	conn, err := pgx.Connect(ctx, "postgres://pgrdf:pgrdf@localhost/pgrdf")
	if err != nil { log.Fatal(err) }
	defer conn.Close(ctx)

	_, err = conn.Exec(ctx, "CREATE EXTENSION IF NOT EXISTS pgrdf")
	if err != nil { log.Fatal(err) }

	// Load a Turtle file
	var n int64
	err = conn.QueryRow(ctx,
		`SELECT pgrdf.load_turtle($1, $2)`,
		"/fixtures/ontologies/foaf.ttl", int64(1),
	).Scan(&n)
	if err != nil { log.Fatal(err) }
	fmt.Printf("loaded %d triples\n", n)

	// Verbose stats — JSONB → map[string]any
	var stats map[string]any
	err = conn.QueryRow(ctx,
		`SELECT pgrdf.load_turtle_verbose($1, $2, $3)`,
		"/fixtures/ontologies/prov.ttl",
		int64(100),
		"http://www.w3.org/ns/prov#",
	).Scan(&stats)
	if err != nil { log.Fatal(err) }
	fmt.Printf("prov.ttl: %v triples in %v ms\n", stats["triples"], stats["elapsed_ms"])

	// SPARQL — each row's value is a map[string]any
	rows, err := conn.Query(ctx,
		`SELECT sparql FROM pgrdf.sparql($1)`,
		`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
		 SELECT ?s ?n WHERE { ?s foaf:name ?n }`,
	)
	if err != nil { log.Fatal(err) }
	defer rows.Close()

	for rows.Next() {
		var binding map[string]any
		if err := rows.Scan(&binding); err != nil { log.Fatal(err) }
		fmt.Printf("%v -> %v\n", binding["s"], binding["n"])
	}
	if rows.Err() != nil { log.Fatal(rows.Err()) }
}
```

## Connection pool

```go
import "github.com/jackc/pgx/v5/pgxpool"

pool, err := pgxpool.New(ctx, "postgres://pgrdf:pgrdf@localhost/pgrdf")
if err != nil { log.Fatal(err) }
defer pool.Close()

rows, err := pool.Query(ctx,
	`SELECT sparql FROM pgrdf.sparql($1)
	  WHERE sparql->>'n' ~* '^a'`,
	`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
	 SELECT ?p ?n WHERE { ?p foaf:name ?n }`,
)
```

The WHERE filter on the JSONB output runs server-side — pgx never
sees rows that don't match.

## Strongly-typed bindings

For fixed-shape SPARQL queries, define a struct and use
`pgx.RowToStructByName` (pgx 5.3+):

```go
type FoafBinding struct {
	S string `json:"s"`
	N string `json:"n"`
}

rows, err := conn.Query(ctx,
	`SELECT sparql FROM pgrdf.sparql($1)`,
	`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
	 SELECT ?s ?n WHERE { ?s foaf:name ?n }`,
)
if err != nil { log.Fatal(err) }

for rows.Next() {
	var raw []byte
	if err := rows.Scan(&raw); err != nil { log.Fatal(err) }
	var fb FoafBinding
	if err := json.Unmarshal(raw, &fb); err != nil { log.Fatal(err) }
	fmt.Println(fb.S, fb.N)
}
```

`raw` is the JSONB column as `[]byte`; `json.Unmarshal` into your
struct gives you per-variable typed fields.

## Bulk ingest pattern

For loading many TTL files, prefer one transaction per file so a
single parse failure doesn't roll back unrelated successes:

```go
for _, path := range paths {
	var n int64
	err := conn.QueryRow(ctx,
		`SELECT pgrdf.load_turtle($1, $2)`,
		path, graphID,
	).Scan(&n)
	if err != nil {
		log.Printf("FAIL %s: %v", path, err)
		continue
	}
	log.Printf("ok %s: %d triples", path, n)
}
```

## Dropping a whole graph (constant-time)

```go
_, err := conn.Exec(ctx,
	`SELECT pgrdf.drop_graph($1)`, graphID,
)
```

`pgrdf.drop_graph` drops the graph's partition **and** removes its
`_pgrdf_graphs` mapping row in one call — use it instead of a raw
`DROP TABLE` on the partition, which would strand the mapping row.
It takes a bind parameter, so no string formatting is needed.

## sqlc + pgrdf

If you're already using sqlc (`https://sqlc.dev`), the SPARQL
returns work fine as `json.RawMessage`:

```yaml
# sqlc.yaml
queries:
  - name: SparqlSelect
    sql: SELECT * FROM pgrdf.sparql($1)
    args: [{ name: query, type: text }]
    return:
      - name: sparql
        type: json.RawMessage
```

You unmarshal the per-row JSON into a struct in application code.

## Type mapping reference

| Postgres type | Go (pgx v5) |
|---|---|
| `BIGINT` (graph id, dict id, triple count) | `int64` |
| `SMALLINT` (term_type) | `int16` |
| `TEXT` (lexical_value, path arg) | `string` |
| `JSONB` (*_verbose return, sparql row) | `map[string]any` or `[]byte` |
| `BOOLEAN` (`add_graph` return) | `bool` |

## Caveats

- pgRDF's strict Turtle parser will reject off-spec TTL. Don't
  swallow parse errors in your client; the source is the bug.
- `pgrdf.sparql` searches the default union of all graphs; scope a
  query with `GRAPH <iri> { … }` or `GRAPH ?g { … }` to target or
  bind named graphs.
- Set `SET search_path = pgrdf, public;` per connection to drop the
  schema prefix on every call.
- `pgrdf.load_turtle` holds the SPI connection for the duration of
  the parse. Use a dedicated pool connection for bulk loads.
