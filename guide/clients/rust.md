# Rust

Both common async PostgreSQL clients work with pgRDF. Examples use the
Docker setup from the [install guide](../01-install.md).

## tokio-postgres

```toml
[dependencies]
tokio-postgres = { version = "0.7", features = ["with-serde_json-1"] }
tokio          = { version = "1", features = ["macros", "rt-multi-thread"] }
serde_json     = "1"
```

```rust
use serde_json::Value;
use tokio_postgres::{error::SqlState, NoTls};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, conn) = tokio_postgres::connect(
        "host=localhost user=postgres password=pgrdf dbname=postgres", NoTls).await?;
    tokio::spawn(async move { if let Err(e) = conn.await { eprintln!("connection: {e}"); } });

    client.batch_execute("CREATE EXTENSION IF NOT EXISTS pgrdf").await?;

    // create a graph and load Turtle
    let graph_id: i64 = client
        .query_one("SELECT pgrdf.add_graph($1)", &[&"http://example.org/people"])
        .await?.get(0);
    let turtle = r#"
        @prefix ex: <http://example.org/> .
        @prefix foaf: <http://xmlns.com/foaf/0.1/> .
        ex:alice foaf:name "Alice" ; foaf:knows ex:bob .
        ex:bob   foaf:name "Bob" .
    "#;
    let n: i64 = client
        .query_one("SELECT pgrdf.parse_turtle($1, $2)", &[&turtle, &graph_id])
        .await?.get(0);
    println!("loaded {n} triples");

    // SPARQL: one JSONB row per solution
    let q = "PREFIX foaf: <http://xmlns.com/foaf/0.1/>
             SELECT ?who ?friend WHERE { ?a foaf:name ?who ; foaf:knows ?b . ?b foaf:name ?friend }";
    for row in client.query("SELECT sparql FROM pgrdf.sparql($1)", &[&q]).await? {
        let b: Value = row.get(0);
        println!("{} → {}", b["who"], b["friend"]);
    }

    // refusals carry a SQLSTATE
    if let Err(e) = client
        .execute("SELECT pgrdf.clear_graph($1::bigint)", &[&graph_id]).await
    {
        if e.code() == Some(&SqlState::LOCK_NOT_AVAILABLE) {
            eprintln!("graph is locked: {e}");
        } else {
            return Err(e.into());
        }
    }
    Ok(())
}
```

## sqlx

```toml
[dependencies]
sqlx       = { version = "0.8", features = ["runtime-tokio", "postgres", "json"] }
tokio      = { version = "1", features = ["macros", "rt-multi-thread"] }
serde      = { version = "1", features = ["derive"] }
serde_json = "1"
```

```rust
use serde::Deserialize;
use sqlx::{postgres::PgPoolOptions, types::Json};

#[derive(Deserialize, Debug)]
struct Binding { s: String, o: String }

#[tokio::main]
async fn main() -> Result<(), sqlx::Error> {
    let pool = PgPoolOptions::new()
        .connect("postgres://postgres:pgrdf@localhost/postgres").await?;

    let (g,): (i64,) = sqlx::query_as("SELECT pgrdf.add_graph($1)")
        .bind("http://example.org/demo").fetch_one(&pool).await?;
    sqlx::query("SELECT pgrdf.parse_turtle($1, $2)")
        .bind(r#"<http://example.org/a> <http://example.org/p> "x" ."#).bind(g)
        .execute(&pool).await?;

    let rows: Vec<(Json<Binding>,)> = sqlx::query_as("SELECT sparql FROM pgrdf.sparql($1)")
        .bind("SELECT ?s ?o WHERE { ?s <http://example.org/p> ?o }")
        .fetch_all(&pool).await?;
    for (Json(b),) in rows { println!("{b:?}"); }
    Ok(())
}
```

## Type mapping

| PostgreSQL | Rust |
|---|---|
| `bigint` (graph ids, counts) | `i64` |
| `text` (IRIs, digests, N-Triples lines) | `String` / `&str` |
| `jsonb` (SPARQL rows, reports) | `serde_json::Value`, or `sqlx::types::Json<T>` |
| `boolean` (`lock_graph`, `unlock_graph`) | `bool` |

## Tips

- Check completeness after a query on the same connection:
  `SELECT pgrdf.last_call_stats()`. Both figures zero means complete.
- `pgrdf.load_turtle(path, …)` reads from the **database server's**
  filesystem. For local files, read them and use `parse_turtle`.
- SQL bind parameters protect the SQL call, not the SPARQL text.
  Escape user input you splice into queries.
