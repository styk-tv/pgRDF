# Go

[`pgx`](https://github.com/jackc/pgx) works with pgRDF like any other
extension: every capability is a SQL function. Examples use the Docker
setup from the [install guide](../01-install.md).

```bash
go get github.com/jackc/pgx/v5
```

```go
package main

import (
	"context"
	"errors"
	"fmt"
	"log"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
)

func main() {
	ctx := context.Background()
	conn, err := pgx.Connect(ctx, "postgres://postgres:pgrdf@localhost:5432/postgres")
	if err != nil { log.Fatal(err) }
	defer conn.Close(ctx)

	if _, err := conn.Exec(ctx, "CREATE EXTENSION IF NOT EXISTS pgrdf"); err != nil { log.Fatal(err) }

	// create a graph and load Turtle
	var graphID int64
	if err := conn.QueryRow(ctx, `SELECT pgrdf.add_graph($1)`,
		"http://example.org/people").Scan(&graphID); err != nil { log.Fatal(err) }

	var n int64
	err = conn.QueryRow(ctx, `SELECT pgrdf.parse_turtle($1, $2)`,
		`@prefix ex: <http://example.org/> .
		 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
		 ex:alice foaf:name "Alice" ; foaf:knows ex:bob .
		 ex:bob   foaf:name "Bob" .`, graphID).Scan(&n)
	if err != nil { log.Fatal(err) }
	fmt.Println("loaded", n, "triples")

	// SPARQL: each row is a JSONB object → map[string]any
	rows, err := conn.Query(ctx, `SELECT sparql FROM pgrdf.sparql($1)`,
		`PREFIX foaf: <http://xmlns.com/foaf/0.1/>
		 SELECT ?who ?friend WHERE { ?a foaf:name ?who ; foaf:knows ?b . ?b foaf:name ?friend }`)
	if err != nil { log.Fatal(err) }
	for rows.Next() {
		var b map[string]any
		if err := rows.Scan(&b); err != nil { log.Fatal(err) }
		fmt.Println(b["who"], "→", b["friend"])
	}
	if err := rows.Err(); err != nil { log.Fatal(err) }

	// refusals carry a SQLSTATE
	_, err = conn.Exec(ctx, `SELECT pgrdf.materialize($1, 'bogus')`, graphID)
	var pgErr *pgconn.PgError
	if errors.As(err, &pgErr) {
		fmt.Println(pgErr.Code, pgErr.Message)   // 22023 materialize: unknown profile ...
	}
}
```

## Typed bindings

For a fixed query shape, scan the JSONB into a struct:

```go
type Person struct {
	Who    string `json:"who"`
	Friend string `json:"friend"`
}

rows, _ := conn.Query(ctx, `SELECT sparql FROM pgrdf.sparql($1)`, query)
people, err := pgx.CollectRows(rows, pgx.RowTo[Person])
```

## Was the answer complete?

Run this on the same connection, right after the query:

```go
var stats map[string]any
conn.QueryRow(ctx, `SELECT pgrdf.last_call_stats()`).Scan(&stats)
complete := stats["path_depth_truncations"] == float64(0) &&
	stats["filter_clauses_dropped"] == float64(0)
```

With a `pgxpool.Pool`, acquire one connection (`pool.Acquire`) for the
query and the stats call so both run in the same session.

## Error codes

| Code | Meaning |
|---|---|
| `55P03` | graph locked |
| `55000` | wrong state (e.g. unlock an unlocked graph) |
| `22023` | invalid argument |
| `0A000` | unsupported construct |
| `42704` | unknown graph |
| `2BP01` | drop without cascade over inferred triples |
| `54000` | configured limit exceeded |

Full list: [errors and diagnostics](../08-errors-and-diagnostics.md).

## Tips

- `pgrdf.load_turtle(path, …)` reads from the **database server's**
  filesystem. For local files, read them and use `parse_turtle`.
- SQL parameters protect the SQL call, not the SPARQL text inside it.
  Escape user input you splice into queries.
- Values in SPARQL results are strings. Convert numbers with `strconv`.
