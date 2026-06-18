# Rust clients

Two mainstream Rust Postgres clients, both work against pgRDF
identically: every capability is a SQL function call.

## tokio-postgres

```toml
# Cargo.toml
tokio-postgres = "0.7"
tokio          = { version = "1", features = ["macros", "rt-multi-thread"] }
serde_json     = "1"   # for parsing the *_verbose JSONB return
```

```rust
use serde_json::Value;
use tokio_postgres::NoTls;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (client, conn) = tokio_postgres::connect(
        "host=localhost user=pgrdf password=pgrdf dbname=pgrdf",
        NoTls,
    ).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await { eprintln!("connection error: {e}"); }
    });

    client.execute("CREATE EXTENSION IF NOT EXISTS pgrdf", &[]).await?;

    // Load a Turtle file.
    let n: i64 = client.query_one(
        "SELECT pgrdf.load_turtle($1, $2)",
        &[&"/fixtures/ontologies/foaf.ttl", &1_i64],
    ).await?.get(0);
    println!("loaded {n} triples");

    // Parse an inline string.
    let n: i64 = client.query_one(
        "SELECT pgrdf.parse_turtle($1, $2)",
        &[&"@prefix ex: <http://e.com/> . ex:a ex:p ex:b .", &2_i64],
    ).await?.get(0);
    println!("parsed {n} triples");

    // Verbose: read JSONB stats into a serde_json::Value.
    let stats: Value = client.query_one(
        "SELECT pgrdf.load_turtle_verbose($1, $2, $3)",
        &[
            &"/fixtures/ontologies/prov.ttl",
            &100_i64,
            &"http://www.w3.org/ns/prov#",
        ],
    ).await?.get(0);
    println!(
        "prov.ttl: {} triples in {} ms ({} cache hits / {} db calls)",
        stats["triples"], stats["elapsed_ms"],
        stats["dict_cache_hits"], stats["dict_db_calls"],
    );

    Ok(())
}
```

## sqlx

```toml
# Cargo.toml
sqlx       = { version = "0.8", features = ["runtime-tokio", "postgres", "json"] }
tokio      = { version = "1",   features = ["macros", "rt-multi-thread"] }
serde_json = "1"
```

```rust
use serde_json::Value;
use sqlx::postgres::PgPoolOptions;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect("postgres://pgrdf:pgrdf@localhost/pgrdf")
        .await?;

    sqlx::query("CREATE EXTENSION IF NOT EXISTS pgrdf").execute(&pool).await?;

    // Load with sqlx's typed binding.
    let (n,): (i64,) = sqlx::query_as(
        "SELECT pgrdf.load_turtle($1, $2)"
    )
    .bind("/fixtures/ontologies/foaf.ttl")
    .bind(1_i64)
    .fetch_one(&pool)
    .await?;
    println!("loaded {n} triples");

    // JSONB stats round-trip via serde_json::Value.
    let stats: (Value,) = sqlx::query_as(
        "SELECT pgrdf.load_turtle_verbose($1, $2, $3)"
    )
    .bind("/fixtures/ontologies/prov.ttl")
    .bind(100_i64)
    .bind("http://www.w3.org/ns/prov#")
    .fetch_one(&pool)
    .await?;
    println!("{:#}", stats.0);

    Ok(())
}
```

## Type mapping

| Postgres type | Rust type (tokio-postgres / sqlx) |
|---|---|
| `BIGINT` (graph id, dict id, triple count) | `i64` |
| `SMALLINT` (term_type) | `i16` |
| `TEXT` (lexical_value, language_tag, path arg) | `&str` / `String` |
| `JSONB` (*_verbose return) | `serde_json::Value` |
| `BOOLEAN` (`add_graph` return) | `bool` |
| `_pgrdf_quads.subject_id`, etc | `i64` |

## Common patterns

**Ingest a batch of files** — kept simple, one transaction per file
so a single broken TTL doesn't abort the whole job:

```rust
for path in turtle_paths {
    match client.execute(
        "SELECT pgrdf.load_turtle($1, $2)",
        &[&path.as_str(), &graph_id],
    ).await {
        Ok(_) => println!("ok: {}", path),
        Err(e) => eprintln!("FAIL: {} → {}", path, e),
    }
}
```

**Drop a whole graph (constant time)** — drops the partition and its
mapping row, not a `DELETE`:

```rust
client.execute(
    "SELECT pgrdf.drop_graph($1)",
    &[&graph_id],
).await?;
```

(Use `pgrdf.drop_graph` rather than a raw `DROP TABLE` on the
partition — the UDF also removes the `_pgrdf_graphs` mapping row, and
it takes a bind parameter so there's no identifier-injection concern.)

## Caveats

- pgRDF runs the parser strictly via `oxttl 0.2`. Anything that
  fails to load is genuinely off-spec. Don't catch parse errors and
  retry with a "lenient" path — there isn't one. Fix the TTL.
- Long-running `load_turtle` calls hold the SPI connection. If
  you're streaming millions of triples, consider issuing one
  `load_turtle` per file so other queries don't queue behind a
  single multi-minute call.
- pgRDF's schema is `pgrdf`. Set `SET search_path = pgrdf, public;`
  once per connection if you want to drop the schema prefix on
  every call.
